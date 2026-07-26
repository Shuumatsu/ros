use buddy_system_allocator::LockedHeap;
use core::alloc::Layout;

use paging::sv39::PAGE_SIZE;

pub mod direct_map;
pub mod frame;
pub mod kernel_table;
pub mod layout;
pub mod region;
pub mod stack;

/// ORDER determines max allocation size: 2^(ORDER-1) bytes
/// ORDER=32 supports up to 2GB allocations
const ORDER: usize = 32;

#[global_allocator]
static HEAP: LockedHeap<ORDER> = LockedHeap::empty();

/// Bytes carved out of the physical frame allocator for the kernel heap.
///
/// Kept bounded on purpose: the *rest* of RAM — the bulk — stays with `frame`,
/// which owns all of physical memory and comes up first. The heap is merely its
/// first customer, and holds only kernel bookkeeping that is not page-shaped;
/// 8 MiB is ample. Prefer `frame` for anything page-sized (page tables, user
/// pages, DMA buffers); grow this only if you must `Box` large buffers.
const KERNEL_HEAP_SIZE: usize = 8 * 1024 * 1024;

/// Translate a physical address to its kernel virtual address (`VA = PA + OFFSET`).
///
/// A compile-time add, and valid for *every* physical address — RAM and MMIO
/// alike — because the kernel's map is linear. See [`direct_map`].
#[inline]
pub const fn phys_to_virt(pa: usize) -> usize {
    pa.wrapping_add(direct_map::VA_OFFSET)
}

/// Translate a kernel direct-map virtual address back to physical
/// (`PA = VA - OFFSET`).
#[inline]
pub const fn virt_to_phys(va: usize) -> usize {
    va.wrapping_sub(direct_map::VA_OFFSET)
}

#[alloc_error_handler]
fn alloc_error(layout: Layout) -> ! {
    panic!(
        "Heap allocation failed: {} bytes with {}-byte alignment",
        layout.size(),
        layout.align()
    );
}

/// Bring up the memory subsystem. **Boot hart only** — see [`init_secondary`].
///
/// Physical frames, then the kernel heap carved out of them, then the kernel page
/// table built from both. The ordering lives here rather than in the caller: it is a
/// property of how these three depend on each other, not something `start` should
/// have to know.
///
/// Order is load-bearing and now the canonical way round. The frame allocator
/// (`frame`) keeps its metadata in a bitmap it reserves from RAM, so it depends
/// on nothing and can own all of RAM from the start; the heap is just its first
/// customer. Both regions are sized from RAM discovered at runtime — the frames
/// from the linker's `_heap_start` (top of the kernel image) to the device-tree
/// RAM top, the heap as a fixed [`KERNEL_HEAP_SIZE`] slice of those frames — so
/// nothing here is a compile-time guess about how much RAM exists.
pub fn init() {
    // Before anything derives an address from the linker symbols: confirm the linker
    // script and Rust agree about the page size and that every section it lays out is
    // page aligned.
    layout::check();

    println!(
        "[memory] direct map: PA 0x0..{:#x} -> VA {:#x}.. ({} GiB)",
        direct_map::WINDOW_END,
        direct_map::VA_OFFSET,
        direct_map::WINDOW_END / (1024 * 1024 * 1024)
    );

    // 1. Physical frames FIRST: [free_start, ram_end). `free_start` is the top
    //    of the kernel image (a high VA); the allocator vends *physical*
    //    addresses, so convert it back to physical. `ram_end` from the device
    //    tree is already physical and was validated by `device_tree::init`.
    let free_start_pa = virt_to_phys(layout::heap_start());
    let ram_end = crate::device_tree::ram_end()
        .expect("device tree RAM region not discovered; call device_tree::init before memory::init");
    assert!(
        free_start_pa < ram_end,
        "kernel image top {free_start_pa:#x} meets/exceeds RAM top {ram_end:#x}; give the VM more RAM"
    );
    frame::init(free_start_pa, ram_end);
    println!(
        "[memory] frames: {:#x}..{:#x} ({} MiB, physical)",
        free_start_pa,
        ram_end,
        (ram_end - free_start_pa) / 1024 / 1024
    );
    frame::self_test();

    // 2. Kernel heap SECOND, carved from the frame allocator. It holds only
    //    kernel bookkeeping; KERNEL_HEAP_SIZE is bounded on purpose — prefer
    //    `frame` for anything page-sized. It is reached through the high-half
    //    mapping (the kernel is linked high). The backing run is never freed:
    //    the heap is permanent, so its `Frames` token is left to drop, which —
    //    since `Frames` has no destructor and we never call `frame::free` on it
    //    — pins those frames for the kernel's lifetime.
    let heap_pages = KERNEL_HEAP_SIZE / PAGE_SIZE;
    let heap_frames =
        frame::alloc_contiguous(heap_pages).expect("no contiguous RAM for the kernel heap");
    let heap_start = phys_to_virt(heap_frames.base().bits());
    let heap_end = heap_start + KERNEL_HEAP_SIZE;
    unsafe {
        HEAP.lock().add_to_heap(heap_start, heap_end);
    }
    println!(
        "[memory] heap:   {:#x}..{:#x} ({} MiB, virtual)",
        heap_start,
        heap_end,
        KERNEL_HEAP_SIZE / 1024 / 1024
    );

    // 3. The real kernel page table LAST: it needs frames for its tree, and it
    //    derives its direct map from what the allocator ended up owning. Replaces
    //    boot.S's blanket-RWX gigapages with per-section rights and W^X.
    kernel_table::init();
}

/// Adopt the boot hart's memory setup. **Secondary harts only.**
///
/// Physical memory and the heap are global and already up; all this hart needs is
/// to stop running on the boot table. It blocks until the boot hart has published
/// the kernel table, so it is also the barrier that keeps a secondary from touching
/// memory before there is any.
///
/// Currently unreachable — nothing calls SBI HSM `hart_start`, so no secondary hart
/// enters the kernel. Its purpose is to make the split explicit, so that when one
/// does, it cannot accidentally re-run [`init`] and re-initialise the allocator over
/// RAM already in use.
pub fn init_secondary() {
    kernel_table::install();
}

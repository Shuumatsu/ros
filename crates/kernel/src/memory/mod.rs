use buddy_system_allocator::LockedHeap;
use core::alloc::Layout;

use paging::sv39::{PAGE_SIZE, page_size_at};
use paging::utils::align_up;

pub mod direct_map;
pub mod frame;
pub mod kernel_table;
pub mod layout;
pub mod region;
pub mod stack;

/// Bytes mapped by one leaf at the middle level.
///
/// The unit the bulk direct map is tiled in, and therefore the grain at which
/// anything placing itself *next to* the direct map has to align. Lives here rather
/// than in one of its two users so they cannot end up with different ideas of it.
pub const SUPERPAGE: usize = page_size_at(1);

/// First virtual address above everything the direct map occupies.
///
/// **The single owner of that boundary.** Whatever wants virtual address space of its
/// own — [`stack`] today, per-thread kernel stacks later — starts here, and
/// [`kernel_table`] refuses to map anything past it. Two modules deriving this
/// separately is exactly how a stack would come to sit inside a live superpage: the
/// two would agree right up until one of them rounded differently.
///
/// Rounded up to a whole [`SUPERPAGE`], so those finer mappings begin a page-table
/// slot of their own rather than landing inside one the bulk direct map covers with a
/// single leaf.
///
/// Only meaningful once [`frame::init`] has run — it is the allocator that decides how
/// much physical memory the kernel owns, and therefore how far the direct map reaches.
pub fn kernel_va_free_start() -> usize {
    let (_, pool_end) = frame::owned_range();
    align_up(phys_to_virt(pool_end), SUPERPAGE)
}

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

/// Print the kernel's static memory layout.
///
/// Lives here rather than in `cpu`, where it used to: every fact it prints comes from
/// [`layout`] or [`stack`], both of which this module owns. A CPU module that imports
/// nothing but memory internals is reporting someone else's business.
pub fn report_layout() {
    println!("kernel image layout:");
    println!("    load base:    {:#x}", layout::memory_start());
    println!("    text:         {:#x}..{:#x}", layout::text_start(), layout::text_end());
    println!("    rodata:       {:#x}..{:#x}", layout::rodata_start(), layout::rodata_end());
    println!("    data:         {:#x}..{:#x}", layout::data_start(), layout::data_end());
    println!("    bss:          {:#x}..{:#x}", layout::bss_start(), layout::bss_end());
    // Where the linker put it, and nothing more: the geometry inside that range —
    // sizes, guards, the secondaries — belongs to `stack` and is printed by
    // `stack::report`, which runs once the secondaries exist.
    println!(
        "    boot stack:   {:#x}..{:#x}",
        layout::boot_stack_start(),
        layout::boot_stack_end()
    );
    // The heap's end is a runtime fact from the device tree, not a linker symbol.
    println!("    heap start:   {:#x}", layout::heap_start());
}

/// Bring up the memory subsystem. **Boot hart only.**
///
/// Physical frames, then the kernel heap carved out of them, then one stack per hart
/// in `secondary_harts`, then the kernel page table built from all of it. The
/// ordering lives here rather than in the caller: it is a property of how these
/// depend on each other, not something `start` should have to know.
///
/// `secondary_harts` is a *parameter* rather than something looked up here, because
/// deciding which harts this kernel will start is `cpu`'s business and `cpu` already
/// depends on this module. Reaching up to ask it would make the dependency circular;
/// `start`, which knows both, supplies it instead.
///
/// A secondary hart runs none of this. It is handed a finished page table and a
/// finished stack by `boot.S` before it reaches Rust at all, which is what makes
/// re-initialising the allocator over live RAM impossible rather than merely
/// discouraged.
///
/// Order is load-bearing and now the canonical way round. The frame allocator
/// (`frame`) keeps its metadata in a bitmap it reserves from RAM, so it depends
/// on nothing and can own all of RAM from the start; the heap is just its first
/// customer. Both regions are sized from RAM discovered at runtime — the frames
/// from the linker's `_heap_start` (top of the kernel image) to the device-tree
/// RAM top, the heap as a fixed [`KERNEL_HEAP_SIZE`] slice of those frames — so
/// nothing here is a compile-time guess about how much RAM exists.
pub fn init(secondary_harts: impl Iterator<Item = usize>) {
    report_layout();

    // Before anything derives an address from the linker symbols: confirm the linker
    // script and Rust agree about the page size and that every section it lays out is
    // page aligned.
    layout::check();
    stack::check_layout();

    // Deliberately called the *boot* window, not "the direct map": this is what
    // boot.S's table covers, and `kernel_table::init` below replaces it a few lines
    // later with device windows plus exactly the RAM the frame allocator owns. The
    // old wording described something that stopped being true within the same
    // function.
    println!(
        "[memory] boot window: PA 0x0..{:#x} -> VA {:#x}.. ({})",
        direct_map::WINDOW_END,
        direct_map::VA_OFFSET,
        crate::utils::ByteSize(direct_map::WINDOW_END)
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
    // From what `frame` owns, not from what it was asked for. `frame::init` aligns
    // both ends and clamps the top to `direct_map::WINDOW_END`, so on a machine with
    // more RAM than the boot window covers the two differ by the whole excess.
    let (pool_start, pool_end) = frame::owned_range();
    println!(
        "[memory] frames: {:#x}..{:#x} ({}, physical)",
        pool_start,
        pool_end,
        crate::utils::ByteSize(pool_end - pool_start)
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
        "[memory] heap:   {:#x}..{:#x} ({}, virtual)",
        heap_start,
        heap_end,
        crate::utils::ByteSize(KERNEL_HEAP_SIZE)
    );

    // 3. Secondary hart stacks THIRD, one per hart in `secondary_harts`. Before the
    //    page table, because that is what maps them: a secondary switches to the
    //    kernel table before it touches its stack, so the mapping has to be there
    //    from the start rather than added afterwards.
    stack::init(secondary_harts);
    stack::report();

    // 4. The real kernel page table LAST: it needs frames for its tree, and it
    //    derives its direct map from what the allocator ended up owning. Replaces
    //    boot.S's blanket-RWX gigapages with per-section rights and W^X.
    kernel_table::init();
}

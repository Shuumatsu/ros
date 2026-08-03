use buddy_system_allocator::LockedHeap;
use core::alloc::Layout;

use paging::sv39::{PAGE_SIZE, page_size_at};
use paging::{MemoryAddr, PhysicalAddr, VirtualAddr};

pub mod direct_map;
pub mod frame;
pub mod kernel_table;
pub mod layout;
pub mod region;
pub mod stack;

/// Bytes mapped by one leaf at the middle level.
///
/// The grain the bulk direct map is tiled in, and therefore the alignment anything
/// placing itself next to the direct map has to respect.
pub const SUPERPAGE: usize = page_size_at(1);

/// First virtual address above everything the direct map occupies.
///
/// Whatever wants virtual address space of its own — [`stack`] today, per-thread
/// kernel stacks later — starts here, and [`kernel_table`] refuses to map anything
/// past it. Rounded up to a whole [`SUPERPAGE`], so those finer mappings begin in a
/// page-table slot of their own rather than inside one the bulk direct map covers
/// with a single leaf.
///
/// Only meaningful once [`frame::init`] has run: the allocator decides how much
/// physical memory the kernel owns, and therefore how far the direct map reaches.
pub fn kernel_va_free_start() -> VirtualAddr {
    let (_, pool_end) = frame::owned_range();
    phys_to_virt(pool_end).align_up(SUPERPAGE)
}

/// Buddy order: allocations up to 2^(ORDER-1) bytes, i.e. 2 GiB.
const ORDER: usize = 32;

#[global_allocator]
static HEAP: LockedHeap<ORDER> = LockedHeap::empty();

/// Bytes carved out of the physical frame allocator for the kernel heap.
///
/// Bounded on purpose: `frame` owns the bulk of RAM, and the heap holds only kernel
/// bookkeeping that is not page-shaped. Prefer `frame` for anything page-sized (page
/// tables, user pages, DMA buffers); grow this only if you must `Box` large buffers.
const KERNEL_HEAP_SIZE: usize = 8 * 1024 * 1024;

/// Translate a physical address to its kernel virtual address (`VA = PA + OFFSET`).
///
/// A compile-time add, and valid for *every* physical address — RAM and MMIO alike —
/// because the kernel's map is linear. See [`direct_map`].
///
/// The types are the whole point of the signature. This is the one place the kernel
/// crosses between the two address spaces, so it is also the only place where mixing
/// them up is possible; taking and returning a bare `usize` made
/// `phys_to_virt(phys_to_virt(pa))` a legal expression.
#[inline]
pub const fn phys_to_virt(pa: PhysicalAddr) -> VirtualAddr {
    VirtualAddr::new(pa.bits().wrapping_add(direct_map::VA_OFFSET))
}

/// Translate a kernel direct-map virtual address back to physical
/// (`PA = VA - OFFSET`).
///
/// Only meaningful for an address *in* the direct map. A kernel stack VA is not —
/// [`stack`] deliberately maps those above it — and the arithmetic here cannot tell,
/// so a caller holding a virtual address from anywhere else must not use this.
#[inline]
pub const fn virt_to_phys(va: VirtualAddr) -> PhysicalAddr {
    PhysicalAddr::new(va.bits().wrapping_sub(direct_map::VA_OFFSET))
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
pub fn report_layout() {
    println!("kernel image layout:");
    println!("    load base:    {:#x}", layout::memory_start());
    println!("    text:         {:#x}..{:#x}", layout::text_start(), layout::text_end());
    println!("    rodata:       {:#x}..{:#x}", layout::rodata_start(), layout::rodata_end());
    println!("    data:         {:#x}..{:#x}", layout::data_start(), layout::data_end());
    println!("    bss:          {:#x}..{:#x}", layout::bss_start(), layout::bss_end());
    // Bounds only. The geometry inside — sizes, guards, the secondaries — belongs to
    // `stack` and is printed by `stack::report` once the secondaries exist.
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
/// in `secondary_harts`, then the kernel page table built from all of it. The order is
/// a property of how these depend on each other, so it lives here rather than in the
/// caller.
///
/// `secondary_harts` is a parameter rather than something looked up here: deciding
/// which harts this kernel starts is `cpu`'s business and `cpu` already depends on
/// this module, so asking it would make the dependency circular. `start` knows both
/// and supplies it.
///
/// A secondary hart runs none of this — `boot.S` hands it a finished page table and a
/// finished stack before it reaches Rust at all.
pub fn init(secondary_harts: impl Iterator<Item = usize>) {
    report_layout();

    // Before anything derives an address from the linker symbols.
    layout::check();
    stack::check_layout();

    println!(
        "[memory] boot map: PA 0x0..{:#x} -> VA {:#x}.. ({})",
        direct_map::DIRECT_MAP_END,
        direct_map::VA_OFFSET,
        crate::utils::ByteSize(direct_map::DIRECT_MAP_END.bits())
    );

    // 1. Physical frames FIRST: [free_start, ram_end). `free_start` is the top of the
    //    kernel image (a high VA); the allocator vends *physical* addresses, so convert
    //    it back. `ram_end` is already physical, validated by `device_tree::init`.
    let free_start_pa = virt_to_phys(layout::heap_start());
    let ram_end = PhysicalAddr::new(
        crate::device_tree::ram_end().expect(
            "device tree RAM region not discovered; call device_tree::init before memory::init",
        ),
    );
    assert!(
        free_start_pa < ram_end,
        "kernel image top {free_start_pa:#x} meets/exceeds RAM top {ram_end:#x}; give the VM more RAM"
    );
    frame::init(free_start_pa, ram_end);
    // What `frame` owns, not what it was asked for: it aligns both ends and clamps the
    // top to the Sv39 direct-map capacity.
    let (pool_start, pool_end) = frame::owned_range();
    println!(
        "[memory] frames: {:#x}..{:#x} ({}, physical)",
        pool_start,
        pool_end,
        crate::utils::ByteSize(pool_end.sub_addr(pool_start))
    );
    frame::self_test();

    // 2. Kernel heap SECOND, carved from the frame allocator and reached through the
    //    high-half mapping. The backing run is never freed: `Frames` has no destructor
    //    and nothing calls `frame::free` on it, which pins the heap for good.
    let heap_pages = KERNEL_HEAP_SIZE / PAGE_SIZE;
    let heap_frames =
        frame::alloc_contiguous(heap_pages).expect("no contiguous RAM for the kernel heap");
    let heap_start = phys_to_virt(heap_frames.base());
    let heap_end = heap_start.add(KERNEL_HEAP_SIZE);
    unsafe {
        // `buddy_system_allocator` speaks bare addresses; this is a genuine exit.
        HEAP.lock().add_to_heap(heap_start.bits(), heap_end.bits());
    }
    println!(
        "[memory] heap:   {:#x}..{:#x} ({}, virtual)",
        heap_start,
        heap_end,
        crate::utils::ByteSize(KERNEL_HEAP_SIZE)
    );

    // 3. Secondary hart stacks THIRD, before the page table, because that is what maps
    //    them: a secondary switches to the kernel table before it touches its stack.
    stack::init(secondary_harts);
    stack::report();

    // 4. The real kernel page table LAST: it needs frames for its tree, and it derives
    //    its direct map from what the allocator ended up owning. Replaces boot.S's
    //    blanket-RWX gigapages with per-section rights and W^X.
    kernel_table::init();
}

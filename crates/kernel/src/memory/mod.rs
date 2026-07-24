use buddy_system_allocator::LockedHeap;
use core::alloc::Layout;
use core::sync::atomic::{AtomicUsize, Ordering};

pub mod frame;
pub mod layout;

/// The kernel VA↔PA offset (`VA = PA + offset`), measured by `boot.S` as the
/// linked virtual address minus the real physical load address and handed to
/// `start`. Recorded here once so it is never hardcoded — the layout's single
/// source is `kernel.ld`, and this is derived from it at boot.
static VA_OFFSET: AtomicUsize = AtomicUsize::new(0);

/// Record the VA↔PA offset `boot.S` derived. Call once, before any translation.
pub fn set_va_offset(offset: usize) {
    VA_OFFSET.store(offset, Ordering::Relaxed);
}

#[inline]
fn va_offset() -> usize {
    VA_OFFSET.load(Ordering::Relaxed)
}

/// ORDER determines max allocation size: 2^(ORDER-1) bytes
/// ORDER=32 supports up to 2GB allocations
const ORDER: usize = 32;

#[global_allocator]
static HEAP: LockedHeap<ORDER> = LockedHeap::empty();

/// Bytes carved off the top of the kernel image for the kernel heap. Kept
/// bounded on purpose: the *rest* of RAM — the bulk — belongs to the physical
/// frame allocator (`frame`), which cannot exist until this heap does (it keeps
/// its free lists here). The heap holds only kernel bookkeeping, `frame`'s
/// `BTreeSet`s included; 8 MiB is ample. Prefer `frame` for anything page-sized;
/// grow this only if you must `Box` large buffers.
const KERNEL_HEAP_SIZE: usize = 8 * 1024 * 1024;

/// Translate a physical address to its kernel virtual address (`VA = PA + OFFSET`).
/// Valid for RAM, which the kernel maps in its high half (and, for now, also
/// identity-maps). The offset's single source is `kernel.ld`'s `_va_offset`.
#[allow(dead_code)]
pub fn phys_to_virt(pa: usize) -> usize {
    pa.wrapping_add(va_offset())
}

/// Translate a kernel virtual address back to physical (`PA = VA - OFFSET`).
pub fn virt_to_phys(va: usize) -> usize {
    va.wrapping_sub(va_offset())
}

#[alloc_error_handler]
fn alloc_error(layout: Layout) -> ! {
    panic!(
        "Heap allocation failed: {} bytes with {}-byte alignment",
        layout.size(),
        layout.align()
    );
}

/// Initialize the memory subsystem: kernel heap first, then the physical frame
/// allocator over the rest of RAM.
///
/// Order is load-bearing. `frame` (buddy) keeps its free lists on the heap, so
/// the heap must exist before we add frames. Both regions are sized from RAM
/// discovered at runtime — the heap from the linker's `_heap_start` up by
/// [`KERNEL_HEAP_SIZE`], the frames from there to the device-tree RAM top — so
/// nothing here is a compile-time guess about how much RAM exists.
pub fn init() {
    // 1. Kernel heap: [_heap_start, _heap_start + KERNEL_HEAP_SIZE).
    // These are high *virtual* addresses (the kernel is linked high); the heap
    // is reached through the kernel's high-half mapping.
    let heap_start = layout::heap_start();
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

    // 2. Physical frames: [heap_end_pa, ram_end). The frame allocator vends
    //    *physical* addresses, so convert the heap top (a VA) back to physical;
    //    `ram_end` from the device tree is already physical. The RAM top was
    //    validated by `device_tree::init` (it panics on an unusable tree).
    let heap_end_pa = virt_to_phys(heap_end);
    let ram_end = crate::device_tree::ram_end()
        .expect("device tree RAM region not discovered; call device_tree::init before memory::init");
    assert!(
        heap_end_pa < ram_end,
        "kernel heap top {heap_end_pa:#x} meets/exceeds RAM top {ram_end:#x}; shrink KERNEL_HEAP_SIZE or give the VM more RAM"
    );
    frame::add_range(heap_end_pa, ram_end);
    println!(
        "[memory] frames: {:#x}..{:#x} ({} MiB, physical)",
        heap_end_pa,
        ram_end,
        (ram_end - heap_end_pa) / 1024 / 1024
    );

    frame::self_test();
}

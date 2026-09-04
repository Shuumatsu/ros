//! Typed access to symbols defined by `kernel.ld`.

use mmu::PAGE_SIZE;
use mmu::{MemoryAddr, VirtualAddr};

/// Declare linker symbols and typed virtual-address accessors.
macro_rules! linker_symbol {
    (
        addresses { $($fn_name:ident => $sym_name:ident),* $(,)? }
        raw { $($(#[$raw_doc:meta])* $raw_name:ident $(as $link_name:literal)?),* $(,)? }
    ) => {
        unsafe extern "C" {
            $(
                #[doc = concat!("Raw `", stringify!($sym_name), "`. Prefer [`", stringify!($fn_name), "`].")]
                pub static $sym_name: u8;
            )*
            $(
                $(#[$raw_doc])*
                $(#[link_name = $link_name])?
                $(#[doc = concat!("Linked as `", $link_name, "`.")])?
                pub static $raw_name: u8;
            )*
        }
        $(
            #[inline]
            pub fn $fn_name() -> VirtualAddr {
                VirtualAddr::new(&raw const $sym_name as usize)
            }
        )*
    };
}

linker_symbol!(
    addresses {
        text_start         => _text_start,
        text_end           => _text_end,
        rodata_start       => _rodata_start,
        rodata_end         => _rodata_end,
        data_start         => _data_start,
        data_end           => _data_end,
        bss_start          => _bss_start,
        bss_end            => _bss_end,
        memory_start       => _memory_start,
        boot_stack_start   => _boot_stack_start,
        boot_stack_end     => _boot_stack_end,
        kernel_top         => _kernel_top,
    }
    raw {
        /// Linker-provided value loaded into `gp`.
        GLOBAL_POINTER as "__global_pointer$",
    }
);

// These declarations are PC-relative symbol addresses, not absolute linker values.

/// Zero `.bss` on the boot hart.
///
/// # Safety
///
/// Call exactly once before any static is accessed.
pub unsafe fn clear_bss() {
    let start = bss_start().bits();
    let end = bss_end().bits();

    // Volatile stores prevent optimization through the one-byte linker symbol declarations.
    let mut addr = start;
    while addr < end {
        unsafe { (addr as *mut usize).write_volatile(0) };
        addr += size_of::<usize>();
    }
}

/// Verify page size and section-alignment invariants before deriving mappings.
///
/// # Panics
///
/// Panics if the linker layout disagrees with Rust's page geometry.
pub fn check() {
    let guard = kernel_top().sub_addr(boot_stack_end());
    assert_eq!(
        guard, PAGE_SIZE,
        "kernel.ld padded {guard:#x} bytes above the boot stack, but Rust's PAGE_SIZE is \
         {PAGE_SIZE:#x}; the linker's _page_size and PAGE_SIZE disagree"
    );

    // Separately protected sections must begin on distinct page boundaries.
    for (name, addr) in [
        ("_memory_start", memory_start()),
        ("_rodata_start", rodata_start()),
        ("_data_start", data_start()),
        ("_bss_start", bss_start()),
        ("_boot_stack_start", boot_stack_start()),
        ("_kernel_top", kernel_top()),
    ] {
        assert!(addr.is_aligned(PAGE_SIZE), "{name} = {addr:#x} is not page aligned");
    }
}

pub fn report() {
    println!("kernel image layout:");
    println!("    load base:    {:#x}", memory_start());
    println!("    text:         {:#x}..{:#x}", text_start(), text_end());
    println!("    rodata:       {:#x}..{:#x}", rodata_start(), rodata_end());
    println!("    data:         {:#x}..{:#x}", data_start(), data_end());
    println!("    bss:          {:#x}..{:#x}", bss_start(), bss_end());
    println!("    boot stack:   {:#x}..{:#x}", boot_stack_start(), boot_stack_end());
    println!("    kernel top:   {:#x}", kernel_top());
}

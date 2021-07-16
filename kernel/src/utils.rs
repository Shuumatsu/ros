use alloc::{format, string::String};
use core::ops::Range;

#[allow(unused_macros)]
#[macro_use]
macro_rules! set_nth_bit {
    ($bits: expr, $n: expr, $b: expr) => {{
        assert!(($n as usize) < core::mem::size_of_val(&$bits) * 8);

        $bits & !(1 << $n) | (if $b { 1 } else { 0 } << $n)
    }};
}

#[allow(unused_macros)]
#[macro_use]
macro_rules! toggle_nth_bit {
    ($bits: expr, $n: expr) => {{
        assert!(($n as usize) < core::mem::size_of_val(&$bits) * 8);

        $bits ^ (1 << $n)
    }};
}

#[allow(unused_macros)]
#[macro_use]
macro_rules! extract_nth_bit {
    ($bits: expr, $n: expr) => {{
        assert!(($n as usize) < core::mem::size_of_val(&$bits) * 8);

        match ($bits >> $n) & 1 {
            0 => false,
            1 => true,
            _ => panic!("unexpected result"),
        }
    }};
}

#[allow(unused_macros)]
#[macro_use]
macro_rules! extract_range {
    ($bits: expr, $start_pos: expr, $end_pos: expr) => {{
        ($start_pos..$end_pos).fold(0, |accu, n| {
            (accu << 1) & if extract_nth_bit!($bits, n) { 1 } else { 0 }
        })
    }};
}

#[allow(unused_macros)]
#[macro_use]
macro_rules! store_range {
    ($bits: expr, $start_pos: expr, $end_pos: expr, $val: expr) => {{
        ($start_pos..$end_pos).fold($bits, |bits, n| {
            let b = extract_nth_bit!($val, n - $start_pos);
            set_nth_bit!(bits, n, b)
        })
    }};
}

#[allow(unused_macros)]
#[macro_use]
macro_rules! set_range {
    ($bits: expr, $start_pos: expr, $end_pos: expr, $b: expr) => {{
        ($start_pos..$end_pos).fold($bits, |bits, n| set_nth_bit!(bits, n, $b))
    }};
}

pub const KILOBYTE: usize = 1024;
pub const MEGABYTE: usize = 1024 * KILOBYTE;
pub const GIGABYTE: usize = 1024 * MEGABYTE;
pub const TERABYTE: usize = 1024 * GIGABYTE;

pub fn format_size(size: usize) -> String {
    if size >= 2 * TERABYTE {
        format!("{} TB", size / TERABYTE)
    } else if size >= 2 * GIGABYTE {
        format!("{} GB", size / GIGABYTE)
    } else if size >= 2 * MEGABYTE {
        format!("{} MB", size / MEGABYTE)
    } else if size >= 2 * KILOBYTE {
        format!("{} KB", size / KILOBYTE)
    } else {
        format!("{} B", size)
    }
}

pub unsafe fn zero_volatile<T>(range: Range<*mut T>)
where
    T: From<u8>,
{
    let mut ptr = range.start;
    while ptr < range.end {
        core::ptr::write_volatile(ptr, T::from(0));
        ptr = ptr.offset(1);
    }
}

/// Align downwards. Returns the greatest x with alignment `align`
/// so that x <= addr. The alignment must be a power of 2.
pub fn align_down(addr: usize, align: usize) -> usize {
    if align.is_power_of_two() {
        addr & !(align - 1)
    } else if align == 0 {
        addr
    } else {
        panic!("`align` must be a power of 2");
    }
}

/// Align upwards. Returns the smallest x with alignment `align`
/// so that x >= addr. The alignment must be a power of 2.
pub fn align_up(addr: usize, align: usize) -> usize {
    align_down(addr + align - 1, align)
}

pub unsafe fn memset(ptr: *mut u8, ch: u8, count: usize) {
    for _ in 0..count {
        *ptr = ch;
    }
}

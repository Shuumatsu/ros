use alloc::{format, string::String};
use core::mem::size_of;
use core::ops::Range;

pub const fn set_nth_bit(bits: usize, n: usize, b: bool) -> usize {
    assert!(n < size_of::<usize>() * 8);

    bits & !(1 << n) | (if b { 1 } else { 0 } << n)
}

pub const fn toggle_nth_bit(bits: usize, n: usize) -> usize {
    assert!(n < size_of::<usize>() * 8);

    bits ^ (1 << n)
}

pub const fn extract_nth_bit(bits: usize, n: usize) -> bool {
    assert!(n < size_of::<usize>() * 8);

    match (bits >> n) & 1 {
        0 => false,
        1 => true,
        _ => panic!("unexpected result"),
    }
}

pub const fn extract_value(bits: usize, mask: usize, start_pos: usize) -> usize {
    assert!(start_pos < size_of::<usize>() * 8);

    (bits & (mask << start_pos)) >> start_pos
}

pub fn set_range(bits: usize, val: usize, start_pos: usize, end_pos: usize) -> usize {
    assert!(start_pos < size_of::<usize>() * 8 && end_pos < size_of::<usize>() * 8);
    assert!(start_pos < end_pos);

    (start_pos..end_pos).fold(bits, |bits, n| {
        let b = extract_nth_bit(val, n - start_pos);
        set_nth_bit(bits, n, b)
    })
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
    println!("{:?}", range);
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
pub fn align_up(addr: usize, align: usize) -> usize { align_down(addr + align - 1, align) }

//! Bit manipulation utilities for paging operations.

use core::mem::size_of;

pub const fn set_nth_bit(bits: usize, n: usize, b: bool) -> usize {
    assert!(n < size_of::<usize>() * 8);

    bits & !(1 << n) | (if b { 1 } else { 0 } << n)
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

/// Check if n is a power of two
#[inline]
pub const fn is_power_of_two(n: usize) -> bool {
    n != 0 && (n & (n - 1)) == 0
}

#[inline]
pub const fn align_down(addr: usize, align: usize) -> usize {
    debug_assert!(is_power_of_two(align), "alignment must be power of 2");
    addr & !(align - 1)
}

#[inline]
pub const fn align_up(addr: usize, align: usize) -> usize {
    debug_assert!(is_power_of_two(align), "alignment must be power of 2");
    (addr + align - 1) & !(align - 1)
}

#[inline]
pub const fn align_offset(addr: usize, align: usize) -> usize {
    debug_assert!(is_power_of_two(align), "alignment must be power of 2");
    addr & (align - 1)
}

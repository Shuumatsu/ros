//! Bit-field and alignment primitives shared across the paging structures.
//!
//! Every structure in this crate is a packed bit-field over a `usize`
//! (addresses and page-table entries alike). These helpers are the single
//! place that knows how to read and write such fields, so no module hard-codes
//! shift/mask arithmetic of its own.

const USIZE_BITS: usize = usize::BITS as usize;

/// A bitmask with the low `width` bits set.
///
/// `width == USIZE_BITS` yields all-ones (guards against a `1 << 64` overflow).
#[inline]
pub const fn mask(width: usize) -> usize {
    debug_assert!(width <= USIZE_BITS, "mask width exceeds usize");
    if width == USIZE_BITS { usize::MAX } else { (1 << width) - 1 }
}

/// Extract the `width`-bit field of `word` that begins at bit `shift`.
#[inline]
pub const fn field(word: usize, shift: usize, width: usize) -> usize {
    debug_assert!(width > 0 && shift + width <= USIZE_BITS, "field out of range");
    (word >> shift) & mask(width)
}

/// Return `word` with its `width`-bit field at `shift` replaced by the low
/// `width` bits of `value`. Bits of `value` outside the field are ignored.
#[inline]
pub const fn with_field(word: usize, shift: usize, width: usize, value: usize) -> usize {
    debug_assert!(width > 0 && shift + width <= USIZE_BITS, "field out of range");
    let m = mask(width) << shift;
    (word & !m) | ((value << shift) & m)
}

pub const KILOBYTE: usize = 1024;
pub const MEGABYTE: usize = 1024 * KILOBYTE;
pub const GIGABYTE: usize = 1024 * MEGABYTE;
pub const TERABYTE: usize = 1024 * GIGABYTE;

#[inline]
pub const fn align_down(addr: usize, align: usize) -> usize {
    debug_assert!(align.is_power_of_two(), "alignment must be power of 2");
    addr & !(align - 1)
}

#[inline]
pub const fn align_up(addr: usize, align: usize) -> usize {
    debug_assert!(align.is_power_of_two(), "alignment must be power of 2");
    (addr + align - 1) & !(align - 1)
}

#[inline]
pub const fn align_offset(addr: usize, align: usize) -> usize {
    debug_assert!(align.is_power_of_two(), "alignment must be power of 2");
    addr & (align - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_widths() {
        assert_eq!(mask(0), 0, "zero-width mask is empty");
        assert_eq!(mask(1), 0b1);
        assert_eq!(mask(9), 0x1FF, "9-bit VPN/PPN index mask");
        assert_eq!(mask(12), 0xFFF, "page offset mask");
        assert_eq!(mask(44), (1usize << 44) - 1, "full PPN mask");
        assert_eq!(mask(USIZE_BITS), usize::MAX, "full-width mask must not overflow");
    }

    #[test]
    fn field_roundtrip() {
        // Write 0x1AB into bits [10..19) of an otherwise-set word, read it back,
        // and confirm no neighbouring bits were disturbed.
        let base = 0xFFFF_FFFF_FFFF_FFFF;
        let w = with_field(base, 10, 9, 0x1AB);
        assert_eq!(field(w, 10, 9), 0x1AB & 0x1FF, "field reads back what was written");
        assert_eq!(field(w, 0, 10), mask(10), "low bits untouched");
        assert_eq!(field(w, 19, 9), mask(9), "high bits untouched");
    }

    #[test]
    fn with_field_truncates_value() {
        // Value wider than the field must be masked, not spill into neighbours.
        let w = with_field(0, 4, 4, 0xFF);
        assert_eq!(w, 0xF0, "only the low 4 bits of 0xFF land in the field");
    }

    #[test]
    fn alignment_helpers() {
        assert_eq!(align_down(0x1FFF, 0x1000), 0x1000);
        assert_eq!(align_up(0x1001, 0x1000), 0x2000);
        assert_eq!(align_up(0x2000, 0x1000), 0x2000, "already aligned is a no-op");
        assert_eq!(align_offset(0x1234, 0x1000), 0x234);
    }
}

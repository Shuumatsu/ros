use core::fmt;
use core::ops::Range;

/// A byte count rendered in the largest binary unit that divides it exactly,
/// falling back to plain bytes.
///
/// Truncating division lies: a bare `/ MIB` at the call site turns 1536 MiB into
/// `1 GiB` and 4095 bytes into `3 KiB`. Kernel sizes are page multiples on every
/// normal path, so the exact-divisor rule prints `8 MiB` where you expect it and
/// surfaces an off-by-one as `4095 B` instead of burying it under a rounded unit.
///
/// ```text
/// ByteSize(8 << 20)    -> "8 MiB"
/// ByteSize(1536 << 20) -> "1536 MiB"   // not "1 GiB"
/// ByteSize(4095)       -> "4095 B"     // not "3 KiB"
/// ByteSize(0)          -> "0 B"
/// ```
#[derive(Clone, Copy)]
pub struct ByteSize(pub usize);

impl fmt::Display for ByteSize {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        const KIB: usize = 1024;
        const MIB: usize = KIB * 1024;
        const GIB: usize = MIB * 1024;
        // Largest first: the first unit that divides the count exactly wins.
        const UNITS: [(usize, &str); 3] = [(GIB, "GiB"), (MIB, "MiB"), (KIB, "KiB")];

        let (scale, unit) = UNITS
            .into_iter()
            .find(|&(scale, _)| self.0 >= scale && self.0.is_multiple_of(scale))
            .unwrap_or((1, "B"));
        write!(f, "{} {unit}", self.0 / scale)
    }
}

pub unsafe fn zero_volatile<T>(range: Range<*mut T>)
where
    T: From<u8>,
{
    let mut ptr = range.start;
    while ptr < range.end {
        unsafe { core::ptr::write_volatile(ptr, T::from(0)) };
        ptr = unsafe { ptr.offset(1) };
    }
}

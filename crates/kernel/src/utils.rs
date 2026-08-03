use core::fmt;

use heapless::String;

/// Copy `name` into a fixed-capacity string, truncating at a character boundary.
///
/// Use this rather than `&name[..name.len().min(N)]`, which slices by **bytes** and
/// panics when the cut lands inside a multi-byte character. Callers name things after
/// device-tree nodes, and node names are firmware input: `fdt-raw` validates them as
/// UTF-8 but does not hold them to the spec's ASCII subset, so a long name with a
/// multi-byte character across the boundary would abort the boot from inside the DTB
/// walk — before the console exists to say so.
///
/// Pushing characters cannot land mid-character, making that failure structurally
/// impossible.
pub fn truncated<const N: usize>(name: &str) -> String<N> {
    let mut out = String::new();
    for c in name.chars() {
        if out.push(c).is_err() {
            break;
        }
    }
    out
}

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

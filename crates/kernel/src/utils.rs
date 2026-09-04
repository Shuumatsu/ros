//! General utility types and functions.

use core::fmt;

use heapless::String;

pub const KIB: usize = 1024;
pub const MIB: usize = 1024 * KIB;
pub const GIB: usize = 1024 * MIB;

/// Copy `name` into a fixed-capacity string, truncating at a character boundary.
pub fn truncated<const N: usize>(name: &str) -> String<N> {
    let mut out = String::new();
    for c in name.chars() {
        if out.push(c).is_err() {
            break;
        }
    }
    out
}

/// Displays a byte count in the largest binary unit that divides it exactly.
#[derive(Clone, Copy)]
pub struct ByteSize(pub usize);

impl fmt::Display for ByteSize {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        const UNITS: [(usize, &str); 3] = [(GIB, "GiB"), (MIB, "MiB"), (KIB, "KiB")];

        let (scale, unit) = UNITS
            .into_iter()
            .find(|&(scale, _)| self.0 >= scale && self.0.is_multiple_of(scale))
            .unwrap_or((1, "B"));
        write!(f, "{} {unit}", self.0 / scale)
    }
}

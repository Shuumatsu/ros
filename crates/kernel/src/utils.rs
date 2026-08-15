//! Small helpers with no subsystem of their own.
//!
//! Both exist because the boot log is read by a human: [`ByteSize`] renders a byte count
//! the way a page multiple should read, and [`truncated`] fits a firmware-supplied name
//! into fixed storage without panicking on it.

use core::fmt;

use heapless::String;

/// Copy `name` into a fixed-capacity string, truncating at a character boundary.
///
/// Not `&name[..N]`, which slices by bytes and panics mid-character. Callers name things
/// after device-tree nodes, which are firmware input and only validated as UTF-8, so that
/// panic would abort the boot from inside the DTB walk — before the console exists.
pub fn truncated<const N: usize>(name: &str) -> String<N> {
    let mut out = String::new();
    for c in name.chars() {
        if out.push(c).is_err() {
            break;
        }
    }
    out
}

/// A byte count in the largest binary unit that divides it *exactly*, else plain bytes.
///
/// Truncating division lies, and kernel sizes are page multiples anyway, so this prints
/// `8 MiB` where you expect it and surfaces an off-by-one as `4095 B` rather than `3 KiB`.
/// `1536 << 20` stays `1536 MiB`, not `1 GiB`.
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

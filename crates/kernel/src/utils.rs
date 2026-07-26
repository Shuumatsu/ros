use alloc::{format, string::String};
use core::mem::size_of;
use core::ops::Range;

/// A byte count rendered in the largest binary unit that divides it, e.g. `8 MiB`.
///
/// One implementation, because there were five: `memory::init` spelled out
/// `/ (1024 * 1024 * 1024)`, `memory::frame` and `device_tree` each did
/// `/ 1024 / 1024`, and `memory::region` had a private `human()`. Every one was the
/// same arithmetic with the unit hardcoded at the call site, so every one silently
/// mislabelled anything outside the magnitude its author had in mind.
pub struct Bytes(pub usize);

impl core::fmt::Display for Bytes {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        const KIB: usize = 1024;
        const MIB: usize = KIB * 1024;
        const GIB: usize = MIB * 1024;
        let (value, unit) = match self.0 {
            bytes if bytes >= GIB => (bytes / GIB, "GiB"),
            bytes if bytes >= MIB => (bytes / MIB, "MiB"),
            bytes if bytes >= KIB => (bytes / KIB, "KiB"),
            bytes => (bytes, "B"),
        };
        write!(f, "{value} {unit}")
    }
}

pub unsafe fn zero_volatile<T>(range: Range<*mut T>)
where
    T: From<u8>,
{
    let mut ptr = range.start;
    println!("{:?}", range);
    while ptr < range.end {
        unsafe { core::ptr::write_volatile(ptr, T::from(0)) };
        ptr = unsafe { ptr.offset(1) };
    }
}

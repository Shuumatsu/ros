use alloc::{format, string::String};
use core::mem::size_of;
use core::ops::Range;

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

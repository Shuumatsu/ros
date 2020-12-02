global_asm!(include_str!("asm/boot.S"));

extern "C" {
    static _text_start: usize;
}
#[inline]
pub fn text_start() -> usize { unsafe { &_text_start as *const _ as _ } }

extern "C" {
    static _text_end: usize;
}
#[inline]
pub fn text_end() -> usize { unsafe { &_text_end as *const _ as _ } }

extern "C" {
    static _global_pointer: usize;
}
#[inline]
pub fn global_pointer() -> usize { unsafe { &_global_pointer as *const _ as _ } }

extern "C" {
    static _rodata_start: usize;
}
#[inline]
pub fn rodata_start() -> usize { unsafe { &_rodata_start as *const _ as _ } }

extern "C" {
    static _rodata_end: usize;
}
#[inline]
pub fn rodata_end() -> usize { unsafe { &_rodata_end as *const _ as _ } }

extern "C" {
    static _data_start: usize;
}
#[inline]
pub fn data_start() -> usize { unsafe { &_data_start as *const _ as _ } }

extern "C" {
    static _data_end: usize;
}
#[inline]
pub fn data_end() -> usize { unsafe { &_data_end as *const _ as _ } }

extern "C" {
    static _bss_start: usize;
}
#[inline]
pub fn bss_start() -> usize { unsafe { &_bss_start as *const _ as _ } }

extern "C" {
    static _bss_end: usize;
}
#[inline]
pub fn bss_end() -> usize { unsafe { &_bss_end as *const _ as _ } }

extern "C" {
    static _memory_start: usize;
}

extern "C" {
    static _kernel_stack_start: usize;
}
#[inline]
pub fn kernel_stack_start() -> usize { unsafe { &_kernel_stack_start as *const _ as _ } }

extern "C" {
    static _kernel_stack_end: usize;
}
#[inline]
pub fn kernel_stack_end() -> usize { unsafe { &_kernel_stack_end as *const _ as _ } }

extern "C" {
    static _heap_start: usize;
}
#[inline]
pub fn heap_start() -> usize { unsafe { &_heap_start as *const _ as _ } }

extern "C" {
    static _heap_size: usize;
}
#[inline]
pub fn heap_size() -> usize { unsafe { &_heap_size as *const _ as _ } }

#[inline]
pub fn memory_start() -> usize { unsafe { &_memory_start as *const _ as _ } }

extern "C" {
    static _memory_end: usize;
}
#[inline]
pub fn memory_end() -> usize { unsafe { &_memory_end as *const _ as _ } }

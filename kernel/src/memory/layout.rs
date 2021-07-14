extern "C" {
    static _memory_start: usize;
}
lazy_static! {
    pub static ref MEMORY_START: usize = unsafe { &_memory_start as *const _ as _ };
}

extern "C" {
    static _memory_end: usize;
}
lazy_static! {
    pub static ref MEMORY_END: usize = unsafe { &_memory_end as *const _ as _ };
}

extern "C" {
    static _text_start: usize;
}
lazy_static! {
    pub static ref TEXT_START: usize = unsafe { &_text_start as *const _ as _ };
}

extern "C" {
    static _text_end: usize;
}
lazy_static! {
    pub static ref TEXT_END: usize = unsafe { &_text_end as *const _ as _ };
}

extern "C" {
    static _rodata_start: usize;
}
lazy_static! {
    pub static ref RODATA_START: usize = unsafe { &_rodata_start as *const _ as _ };
}

extern "C" {
    static _rodata_end: usize;
}
lazy_static! {
    pub static ref RODATA_END: usize = unsafe { &_rodata_end as *const _ as _ };
}

extern "C" {
    static _data_start: usize;
}
lazy_static! {
    pub static ref DATA_START: usize = unsafe { &_data_start as *const _ as _ };
}

extern "C" {
    static _data_end: usize;
}
lazy_static! {
    pub static ref DATA_END: usize = unsafe { &_data_end as *const _ as _ };
}

extern "C" {
    static _bss_start: usize;
}
lazy_static! {
    pub static ref BSS_START: usize = unsafe { &_bss_start as *const _ as _ };
}

extern "C" {
    static _bss_end: usize;
}
lazy_static! {
    pub static ref BSS_END: usize = unsafe { &_bss_end as *const _ as _ };
}

extern "C" {
    static _kernel_stack_start: usize;
}
lazy_static! {
    pub static ref KERNEL_STACK_START: usize = unsafe { &_kernel_stack_start as *const _ as _ };
}

extern "C" {
    static _kernel_stack_end: usize;
}
lazy_static! {
    pub static ref KERNEL_STACK_END: usize = unsafe { &_kernel_stack_end as *const _ as _ };
}

extern "C" {
    static _heap_start: usize;
}
lazy_static! {
    pub static ref HEAP_START: usize = unsafe { &_heap_start as *const _ as _ };
}

extern "C" {
    static _heap_end: usize;
}
lazy_static! {
    pub static ref HEAP_END: usize = unsafe { &_heap_end as *const _ as _ };
}

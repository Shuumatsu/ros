// use alloc::alloc::{alloc, alloc_zeroed, dealloc, Layout};
// use lazy_static::lazy_static;
// use spin::Mutex;

// mod allocator;

// use crate::{arch::riscv64::paging::sv39::map, memory::paging::map_range, trap::TrapFrame};
// use crate::{arch::riscv64::paging::sv39::unmap, memory::paging::Table};
// use crate::{
//     arch::riscv64::paging::sv39::READ_WRITE,
//     utils::{KILOBYTE, MEGABYTE},
// };
// use allocator::PidAllocator;

// pub enum ProcessState {
//     Running,
//     Sleeping,
//     Waiting,
//     Dead,
// }

// #[repr(C)]
// pub struct Process {
//     frame: TrapFrame,
//     stack: *mut u8,
//     program_counter: usize,
//     pid: u16,
//     page_table: *mut Table,
//     state: ProcessState,
// }

// const STACK_LAYOUT: Layout = Layout::from_size_align(4 * MEGABYTE, 4 * KILOBYTE).unwrap();

// lazy_static! {
//     static ref PID_ALLOCATOR: Mutex<PidAllocator> = {
//         let pid_allocator = PidAllocator::new();
//         for pid in 0..=u16::MAX {
//             pid_allocator.dealloc(pid);
//         }
//         Mutex::from(pid_allocator)
//     };
// }

// impl Process {
//     pub fn new_default(func: fn()) -> Option<Self> {
//         let func_addr = func as usize;

//         let stack = unsafe { alloc(STACK_LAYOUT) };
//         if stack.is_null() {
//             return None;
//         }

//         let page_table = unsafe { alloc_zeroed(Layout::new::<Table>()) as *mut Table };
//         if page_table.is_null() {
//             return None;
//         }

//         let pid = {
//             match PID_ALLOCATOR.lock().alloc() {
//                 Some(pid) => pid,
//                 None => return None,
//             }
//         };

//         let frame = TrapFrame::default();
//         frame.sp = stack as usize + STACK_LAYOUT.size();

//         map_range(page_table, stack as usize, frame.sp, f, READ_WRITE);

//         let mut ret_proc = Process { frame, state: ProcessState::Waiting, page_table, stack, pid };

//         None
//     }
// }

// impl Drop for Process {
//     fn drop(&mut self) {
//         unsafe {
//             dealloc(self.stack, STACK_LAYOUT);
//             unmap(self.page_table);
//             dealloc(self.page_table as *mut _, Layout::new::<Table>())
//         }
//     }
// }

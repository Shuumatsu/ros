use core::cell::RefCell;

use lazy_static::*;
use spin::Mutex;

use trapframe::TrapFrame;

const USER_STACK_SIZE: usize = 4096;
const KERNEL_STACK_SIZE: usize = 4096;
const MAX_APP_NUM: usize = 16;
const APP_BASE_ADDRESS: usize = 0x80400000;
const APP_SIZE_LIMIT: usize = 0x20000;

static KERNEL_STACK: [u8; KERNEL_STACK_SIZE] = [0; KERNEL_STACK_SIZE];
static USER_STACK: [u8; USER_STACK_SIZE] = [0; USER_STACK_SIZE];

global_asm!(include_str!("link_app.S"));

pub fn is_valid_location(loc: usize) -> bool {
    let user_stack_loc = &USER_STACK as *const _ as _;

    (loc >= APP_BASE_ADDRESS && loc < APP_BASE_ADDRESS + APP_SIZE_LIMIT)
        || (loc >= user_stack_loc && loc < user_stack_loc + core::mem::size_of_val(&USER_STACK))
}

struct AppManager {
    num_app: usize,
    current_app: usize,
    app_start: [usize; MAX_APP_NUM + 1],
}

impl AppManager {
    unsafe fn load_app(&self, app_id: usize) {
        println!("[Kernel] Loading app_{}", app_id);

        llvm_asm!("fence.i" :::: "volatile");
        (APP_BASE_ADDRESS..APP_BASE_ADDRESS + APP_SIZE_LIMIT).for_each(|addr| {
            (addr as *mut u8).write_volatile(0);
        });
    }
}

lazy_static! {
    static ref APP_MANAGER: Mutex<AppManager> = Mutex::new(unimplemented!());
}

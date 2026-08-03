pub mod software;
pub mod timer;

pub unsafe fn init() {
    unsafe {
        timer::init();
        software::init();
    }
}

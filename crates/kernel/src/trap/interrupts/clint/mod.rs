pub mod software;
pub mod timer;

pub unsafe fn init() {
    timer::init();
    software::init();
}

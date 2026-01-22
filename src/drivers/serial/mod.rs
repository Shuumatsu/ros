use super::Driver;

pub trait SerialDriver: Driver {
    // read one byte from tty
    fn read(&self) -> u8;

    // write bytes to tty
    fn write(&self, data: &[u8]);
}

use core::convert::TryInto;
use core::fmt::{Error, Write};
use lazy_static::lazy_static;
use spin::Mutex;

pub struct UartDriver {
    base_address: usize,
}

// QEMU emulates the NS16550A UART chipset.
// We control this UART system using memory mapped I/O at base address 0x1000_0000.
// we can control the NS16550a registers, the transmitter (THR) and receiver (RBR) registers are both exactly 8-bits
pub const UART_BASE_ADDR: usize = 0x1000_0000;

pub const RHR_OFFSET: usize = 0; // receive holding register (for input bytes)
pub const THR_OFFSET: usize = 0; // transmit holding register (for output bytes)

pub const IER_OFFSET: usize = 1; // interrupt enable register
pub const IER_TX_ENABLE: u8 = 1 << 0;
pub const IER_RX_ENABLE: u8 = 1 << 1;

pub const FCR_OFFSET: usize = 2; // FIFO control register
pub const FCR_FIFO_ENABLE: u8 = 1 << 0;
pub const FCR_FIFO_CLEAR: u8 = 3 << 1; // clear the content of the two FIFOs

pub const ISR_OFFSET: usize = 2; // interrupt status register

pub const LCR_OFFSET: usize = 3; // line control register
pub const LCR_EIGHT_BITS: u8 = 3 << 0;
pub const LCR_BAUD_LATCH: u8 = 1 << 7; // special mode to set baud rate

pub const LSR_OFFSET: usize = 5; // line status register
pub const LSR_RX_READY: u8 = 1 << 0; // input is waiting to be read from RHR
pub const LSR_TX_IDLE: u8 = 1 << 5; // THR can accept another character to send

lazy_static! {
    pub static ref UART_DRIVER: Mutex<UartDriver> = Mutex::new({
        let mut driver = UartDriver::new(UART_BASE_ADDR);
        driver.init();
        driver
    });
}

// the UART control registers; http://byterunner.com/16550.html
impl UartDriver {
    pub fn new(base_address: usize) -> Self { UartDriver { base_address } }

    pub fn init(&mut self) {
        let ptr = self.base_address as *mut u8;
        unsafe {
            // disable interrupts
            ptr.add(IER_OFFSET).write_volatile(0);

            // special mode to set baud rate.
            ptr.add(LCR_OFFSET).write_volatile(LCR_BAUD_LATCH);
            // LSB for baud rate of 38.4K.
            ptr.add(0).write_volatile(0x03);
            // MSB for baud rate of 38.4K.
            ptr.add(1).write_volatile(0x00);

            // leave set-baud mode, and set the transmitter and receiver buffers word length to 8 bits
            ptr.add(LCR_OFFSET).write_volatile(LCR_EIGHT_BITS);

            // reset and enable FIFOs
            ptr.add(FCR_OFFSET).write_volatile(FCR_FIFO_ENABLE | FCR_FIFO_CLEAR);

            // enable transmit and receive interrupts.
            ptr.add(IER_OFFSET).write_volatile(IER_TX_ENABLE | IER_RX_ENABLE);
        }
    }

    // when read from a pointer pointing to 0x1000_0000, we extract those 8-bits from the RBR.
    pub fn put_sync(&mut self, c: u8) {
        let ptr = self.base_address as *mut u8;

        unsafe {
            while (ptr.add(LSR_OFFSET).read_volatile() & LSR_TX_IDLE) == 0 {}

            ptr.add(THR_OFFSET).write_volatile(c);
        }
    }

    // when write to the exact same pointer pointing to 0x1000_0000, we will be transmitting to the THR.
    pub fn get(&mut self) -> Option<u8> {
        let ptr = self.base_address as *mut u8;

        unsafe {
            if ptr.add(LSR_OFFSET).read_volatile() & 1 == 0 {
                // the DR bit is 0, meaning no data
                None
            } else {
                // the DR bit is 1, meaning data!
                Some(ptr.add(RHR_OFFSET).read_volatile())
            }
        }
    }
}

impl Write for UartDriver {
    fn write_str(&mut self, out: &str) -> Result<(), Error> {
        for c in out.bytes() {
            self.put_sync(c);
        }
        Ok(())
    }
}

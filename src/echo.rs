use crate::uart::UART_DRIVER;
use crate::{print, println};

pub fn echo() -> ! {
    loop {
        match {
            let mut driver = UART_DRIVER.lock();
            driver.get()
        } {
            Some(8) => {
                // backspace really means to "move the cursor to the left by 1 character"
                // the cursor would move, but the underlying text would still be there.
                // so, our solution is to move the cursor, draw a space and then move the cursor left by that one space.
                print!("{}{}{}", 8 as char, ' ', 8 as char);
            }
            Some(10) | Some(13) => {
                println!();
            }
            Some(0x1b) => {
                match {
                    let mut driver = UART_DRIVER.lock();
                    driver.get()
                } {
                    Some(91) => {
                        match {
                            let mut driver = UART_DRIVER.lock();
                            driver.get().map(|u| u as char)
                        } {
                            Some('A') => {
                                println!("That's the up arrow!");
                            }
                            Some('B') => {
                                println!("That's the down arrow!");
                            }
                            Some('C') => {
                                println!("That's the right arrow!");
                            }
                            Some('D') => {
                                println!("That's the left arrow!");
                            }
                            _ => {
                                println!("That's something else.....");
                            }
                        }
                    }
                    _ => (),
                }
            }
            Some(c) => {
                print!("{}", c as char);
            }
            None => (),
        }
    }
}

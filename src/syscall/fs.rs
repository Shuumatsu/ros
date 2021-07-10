use crate::batch::is_valid_location;

const FD_STDOUT: usize = 1;

pub fn sys_write(fd: usize, buf: *const u8, len: usize) -> isize {
    match fd {
        FD_STDOUT => {
            if !is_valid_location(buf as usize) || !is_valid_location(buf as usize + len) {
                println!("[kernel] buf out of range");
                return -1;
            }

            let slice = unsafe { core::slice::from_raw_parts(buf, len) };
            let str = core::str::from_utf8(slice).unwrap();
            print!("{}", str);
            len as isize
        }
        _ => {
            println!("[kernel] Unsupported fd in sys_write!");
            -1
        }
    }
}

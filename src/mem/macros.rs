// macro_rules! read_attr {
//     ($read_field: ident, $pos: expr) => {
//         #[inline]
//         pub fn $read_field(&mut self) {
//             self.bits.get_bit($pos);
//         }
//     };
// }

// macro_rules! set_attr {
//     ($set_field: ident, $pos: expr) => {
//         #[inline]
//         pub fn $set_field(&mut self) {
//             self.bits.set_bit($pos, true);
//         }
//     };
// }

// macro_rules! clear_attr {
//     ($clear_field: ident, $pos: expr) => {
//         #[inline]
//         pub fn $clear_field(&mut self) {
//             self.bits.set_bit($pos, false);
//         }
//     };
// }

// macro_rules! get_set_clear_attr {
//     ($read_field: ident, $set_field: ident, $clear_field: ident, $pos: expr) => {
//         read_attr!($read_field, $pos);
//         set_attr!($set_field, $pos);
//         clear_attr!($clear_field, $pos);
//     };
// }

// macro_rules! read_composite_attr {
//     ($read_field: ident, $($bits: expr)+) => {
//         /// Reads the CSR as a 64-bit value
//         #[inline]
//         pub fn read_field() -> bool {
//             self.bits.get_bit
//         }
//     };
// }

// get_set_clear_attr!(is_valid, set_valid, clear_valid, 0);
// get_set_clear_attr!(is_readable, set_readable, clear_readable, 1);
// get_set_clear_attr!(is_writable, set_writable, clear_writable, 2);
// get_set_clear_attr!(is_executable, set_executable, clear_executable, 3);
// get_set_clear_attr!(is_user, set_user, clear_user, 4);
// get_set_clear_attr!(is_global, set_global, clear_global, 5);
// get_set_clear_attr!(is_accessible, set_accessible, clear_accessible, 6);
// get_set_clear_attr!(is_dirty, set_dirty, clear_dirty, 7);

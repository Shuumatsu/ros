pub mod sv39;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mode {
    Bare = 0,
    Sv39 = 8,
    Sv48 = 9,
    Sv57 = 10,
    Sv64 = 11,
}
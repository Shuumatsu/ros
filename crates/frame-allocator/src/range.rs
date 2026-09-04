//! Non-empty frame ranges and their aligned buddy-root decomposition.

use core::cmp::min;

/// A non-empty, half-open range of numeric frame identifiers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameRange {
    start: usize,
    end: usize,
}

impl FrameRange {
    pub const fn new(start: usize, end: usize) -> Result<Self, RangeError> {
        if start < end { Ok(Self { start, end }) } else { Err(RangeError { start, end }) }
    }

    pub const fn start(self) -> usize { self.start }

    pub const fn end(self) -> usize { self.end }

    pub const fn len(self) -> usize { self.end - self.start }

    /// Always false because construction rejects empty ranges.
    pub const fn is_empty(self) -> bool { false }

    pub const fn contains(self, frame: usize) -> bool { self.start <= frame && frame < self.end }

    pub(crate) const fn roots(self) -> RootIter { RootIter { current: self.start, end: self.end } }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("frame range must be non-empty and increasing, got {start}..{end}")]
pub struct RangeError {
    start: usize,
    end: usize,
}

impl RangeError {
    pub const fn start(self) -> usize { self.start }

    pub const fn end(self) -> usize { self.end }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RootBlock {
    pub(crate) start: usize,
    pub(crate) order: usize,
}

impl RootBlock {
    pub(crate) const fn frame_count(self) -> usize { 1usize << self.order }
}

pub(crate) struct RootIter {
    current: usize,
    end: usize,
}

impl Iterator for RootIter {
    type Item = RootBlock;

    fn next(&mut self) -> Option<Self::Item> {
        if self.current == self.end {
            return None;
        }

        let remaining = self.end - self.current;
        let largest_fitting = highest_power_of_two(remaining);
        let alignment = if self.current == 0 {
            largest_fitting
        } else {
            1usize << self.current.trailing_zeros()
        };
        let frame_count = min(alignment, largest_fitting);
        let root = RootBlock { start: self.current, order: frame_count.trailing_zeros() as usize };
        self.current += frame_count;
        Some(root)
    }
}

fn highest_power_of_two(value: usize) -> usize {
    debug_assert_ne!(value, 0);
    1usize << (usize::BITS - 1 - value.leading_zeros())
}

//! Named physical ranges and page-rounded geometry.

use alloc::vec::Vec;
use core::fmt;

use heapless::String;

use mmu::PAGE_SIZE;
use mmu::{MemoryAddr, PhysicalAddr};

use crate::utils::truncated;

/// Maximum retained label length.
pub const NAME_LEN: usize = 40;

/// A named physical address range.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhysRange {
    name: String<NAME_LEN>,
    pub base: PhysicalAddr,
    pub size: usize,
}

impl PhysRange {
    /// Build a range, truncating an overlong label.
    pub fn new(name: &str, base: PhysicalAddr, size: usize) -> Self {
        Self { name: truncated(name), base, size }
    }

    /// Build a range whose label is formatted, keeping as much of it as fits.
    pub fn labeled(name: fmt::Arguments<'_>, base: PhysicalAddr, size: usize) -> Self {
        let mut label = String::new();
        let _ = fmt::Write::write_fmt(&mut label, name);
        Self { name: label, base, size }
    }

    pub fn name(&self) -> &str { &self.name }

    /// Saturating exclusive end.
    pub fn end(&self) -> PhysicalAddr {
        PhysicalAddr::new(self.base.bits().saturating_add(self.size))
    }

    pub fn contains(&self, address: PhysicalAddr) -> bool {
        self.base <= address && address < self.end()
    }

    pub fn overlaps(&self, other: &Self) -> bool {
        self.base < other.end() && other.base < self.end()
    }

    /// Range rounded outward to whole frames.
    pub fn footprint(&self) -> (PhysicalAddr, PhysicalAddr) {
        self.base.footprint(self.end(), PAGE_SIZE)
    }
}

/// Merge a list of physical windows into one whose entries cannot share a page.
///
/// Page-rounded overlaps are merged; adjacent windows remain separate.
pub fn coalesce(windows: &[PhysRange]) -> Vec<PhysRange> {
    struct Run<'a> {
        name: &'a str,
        start: PhysicalAddr,
        end: PhysicalAddr,
        joined: usize,
    }

    let mut sorted: Vec<&PhysRange> = Vec::with_capacity(windows.len());
    sorted.extend(windows.iter().filter(|window| window.size > 0));
    sorted.sort_unstable_by_key(|window| window.base);

    let mut runs: Vec<Run<'_>> = Vec::with_capacity(sorted.len());
    for window in sorted {
        let (start, end) = window.footprint();
        match runs.last_mut() {
            Some(run) if start < run.end => {
                run.end = run.end.max(end);
                run.joined += 1;
            }
            _ => runs.push(Run { name: window.name(), start, end, joined: 0 }),
        }
    }

    runs.into_iter()
        .map(|run| {
            let size = run.end.sub_addr(run.start);
            match run.joined {
                0 => PhysRange::new(run.name, run.start, size),
                joined => {
                    PhysRange::labeled(format_args!("{} +{joined}", run.name), run.start, size)
                }
            }
        })
        .collect()
}

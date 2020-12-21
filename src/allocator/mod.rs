use crate::collections::memlist::{MemList, Node};
use crate::utils::{align_down, align_up};
use crate::{kprint, kprintln};
use core::alloc::Layout;
use core::fmt;
use core::mem::size_of;

const MIN_ALLOCATION_SIZE_ORDER: usize =
    size_of::<Node>().next_power_of_two().trailing_zeros() as usize;
const MAX_ALLOCATION_SIZE_ORDER: usize = 31;
const BUCKET_COUNT: usize = MAX_ALLOCATION_SIZE_ORDER + 1;

#[derive(Debug)]
pub struct Allocator {
    total: usize,
    allocated: usize,
    blocks: [MemList; BUCKET_COUNT],
}
unsafe impl Send for Allocator {}

impl Allocator {
    pub unsafe fn new(start: usize, end: usize) -> Self {
        let mut ret = Allocator { blocks: [MemList::new(); BUCKET_COUNT], total: 0, allocated: 0 };
        ret.add_to_heap(start, end);

        ret
    }

    // start: inclusive; end: exclusive
    pub unsafe fn add_to_heap(&mut self, mut start: usize, mut end: usize) {
        start = align_up(start, size_of::<MemList>());
        end = align_down(end, size_of::<MemList>());
        kprintln!("[add_to_heap] add [{:#x}, {:#x}) to allocator", start, end);

        assert!(start <= end);
        let mut remaining: usize = end - start;

        let mut curr_start = start;
        for order in (MIN_ALLOCATION_SIZE_ORDER..=MAX_ALLOCATION_SIZE_ORDER).rev() {
            let size = 2_usize.pow(order as u32);
            let count = remaining / size;
            remaining = remaining % size;

            for _ in 0..count {
                self.insert_block(curr_start as *mut u8, order);
                self.total += size;

                curr_start += size;
            }
        }
    }

    pub unsafe fn alloc(&mut self, layout: Layout) -> *mut u8 {
        let size = core::cmp::max(
            layout.size().next_power_of_two(),
            core::cmp::max(layout.align(), size_of::<MemList>()),
        );
        let target_order = size.trailing_zeros() as usize;

        let mut os = (target_order..=MAX_ALLOCATION_SIZE_ORDER)
            .filter(|&order| !self.blocks[order].is_empty());
        match os.next() {
            None => 0 as *mut u8,
            Some(first_enough) => {
                let block = self.blocks[first_enough].pop().unwrap();

                let mut curr_start = block as usize;
                for order in target_order..first_enough {
                    let size = 2_usize.pow(order as u32);
                    self.blocks[order].push(curr_start as *mut u8);
                    curr_start += size;
                }

                self.allocated += 2_usize.pow(target_order as u32);
                // kprintln!("[alloc] after alloc for {:?} \n\t{:?}", layout, self);
                // kprintln!("[alloc] allocated at {:#x}", curr_start);
                curr_start as *mut u8
            }
        }
    }

    unsafe fn insert_block(&mut self, ptr: *mut u8, order: usize) {
        let neighbor = self.blocks[order]
            .iter_mut()
            .filter(|node| {
                *node as usize == ptr as usize + 2_usize.pow(order as u32)
                    || ptr as usize == *node as usize + 2_usize.pow(order as u32)
            })
            .next();

        match neighbor {
            None => self.blocks[order].push(ptr),
            Some(node) => {
                self.blocks[order].remove(&mut *node);
                self.insert_block(core::cmp::min(ptr, node as *mut u8), order + 1);
            }
        }
    }

    pub unsafe fn dealloc(&mut self, ptr: *mut u8, layout: Layout) {
        // kprintln!("[dealloc] before dealloc \n\t{:?}", self);
        let size = core::cmp::max(
            layout.size().next_power_of_two(),
            core::cmp::max(layout.align(), size_of::<MemList>()),
        );
        let target_order = size.trailing_zeros() as usize;

        self.insert_block(ptr, target_order);
        self.allocated -= size;

        // kprintln!("[dealloc] after dealloc \n\t{:?}", self);
    }
}

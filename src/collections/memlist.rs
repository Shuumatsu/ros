use core::iter;

#[repr(C)]
#[derive(Debug)]
pub struct Node {
    prev: Option<*mut Node>,
    next: Option<*mut Node>,
}

impl Node {
    pub unsafe fn isolate(&mut self) {
        if let Some(prev) = self.prev {
            (*prev).next = self.next;
        }
        if let Some(next) = self.next {
            (*next).prev = self.prev;
        }
    }
}

#[derive(Debug, Copy, Clone)]
#[repr(transparent)]
pub struct MemList(Option<*mut Node>);
unsafe impl Send for MemList {}

impl MemList {
    pub const fn new() -> Self { MemList(None) }

    pub fn is_empty(&self) -> bool { self.0.is_none() }

    pub fn pop(&mut self) -> Option<*mut Node> {
        let ret = self.0;

        if let Some(node) = ret {
            unsafe { self.remove(&mut *node) };
        }

        ret
    }

    pub unsafe fn push(&mut self, ptr: *mut u8) {
        let node = ptr as *mut Node;
        (*node).prev = None;
        (*node).next = self.0;

        self.0 = Some(node);
    }

    pub unsafe fn remove(&mut self, node: &mut Node) {
        if self.0 == Some(node) {
            self.0 = node.next;
        }

        node.isolate();
    }

    pub fn iter_mut(&mut self) -> IterMut { IterMut { curr: self.0 } }
}

pub struct IterMut {
    curr: Option<*mut Node>,
}

impl iter::Iterator for IterMut {
    type Item = *mut Node;

    fn next(&mut self) -> Option<<Self as Iterator>::Item> {
        match self.curr {
            None => None,
            Some(curr) => {
                self.curr = unsafe { (*curr).next };
                Some(curr)
            }
        }
    }
}

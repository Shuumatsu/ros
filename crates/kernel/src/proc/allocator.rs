use alloc::boxed::Box;

#[derive(Debug)]
struct Node {
    val: u16,
    next: Option<Box<Node>>,
}

#[derive(Debug)]
pub struct PidAllocator(Option<Box<Node>>);

impl PidAllocator {
    pub fn new() -> Self { PidAllocator(None) }

    pub fn alloc(&mut self) -> Option<u16> {
        let ret = self.0.map(|node| node.val);
        if let Some(node) = self.0 {
            self.0 = node.next;
        }
        ret
    }

    pub fn dealloc(&mut self, pid: u16) {
        let node = Node { val: pid, next: self.0 };
        self.0 = Some(Box::from(node));
    }
}

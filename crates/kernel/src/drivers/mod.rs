//! Device drivers, one module per piece of hardware.
//!
//! A driver owns both halves of binding to its device: the `compatible` strings the device
//! tree names it by, and the register access that follows. Together in one module is what
//! lets [`crate::device_tree`] resolve a node without knowing what drives it, and what makes
//! supporting another chip an edit in one file rather than in the tree walk as well.

pub mod uart16550;

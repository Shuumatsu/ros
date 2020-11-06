pub mod paging;

use lazy_static::lazy_static;
use spin::Mutex;

struct FreeList {
    // next: Box<FreeList>,
}

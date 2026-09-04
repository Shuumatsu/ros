//! User-process loading and execution.

pub mod image;

use mmu::VirtualAddr;

use crate::arch::context::{self, KernelContext};
use crate::arch::interrupts;
use crate::arch::trap::{self, TrapFrame};
use crate::cpu;
use crate::memory::address_space::AddressSpace;
use crate::memory::{self, kernel_table, user_table};

static HELLO: &[u8] = include_bytes!(env!("CARGO_BIN_FILE_HELLO"));

/// SIGSEGV-compatible status used when the kernel kills a process.
const FAULT_STATUS: usize = 128 + 11;

/// Running-process state kept on [`run`]'s suspended stack frame.
struct Process {
    space: AddressSpace,
    entry: VirtualAddr,
    resume: KernelContext,
    user_ticks: u64,
    status: usize,
}

/// Load the embedded program, run it to completion, and answer with the status it exited on.
///
/// Blocks until process exit. Process allocations are not reclaimed.
///
/// # Panics
///
/// Panics if the embedded image is invalid or process allocation fails.
pub fn run() -> usize {
    // Allocate the kernel stack before the process table snapshots the kernel mappings.
    let stack = memory::alloc_kernel_stack("process kernel stack");

    let image = image::parse(HELLO)
        .unwrap_or_else(|error| panic!("the embedded user image will not load: {error}"));
    let space = user_table::build(&image.segments, &stack);

    let mut process = Process {
        space,
        entry: image.entry,
        resume: KernelContext::default(),
        user_ticks: 0,
        status: 0,
    };
    let control_block = &raw mut process as usize;
    let first = KernelContext::new(enter, stack.top(), control_block);

    let cpu = cpu::current();
    cpu.enter_process(control_block, stack.top());
    println!(
        "[process] hello: entry {:#x}, satp {:#x}, kernel stack top {:#x}",
        process.entry,
        process.space.satp().bits(),
        stack.top()
    );

    // SAFETY: the space maps this hart's current kernel PC and stack through its shared upper half.
    unsafe { process.space.activate() };

    // Context switching does not preserve CSRs; restore this context's interrupt state on return.
    interrupts::without(|| {
        // SAFETY: `first` begins on the stack allocated above, mapped on this hart, and `exit`
        // switches back into the context saved here.
        unsafe { context::switch(&raw mut process.resume, &raw const first) };
    });

    kernel_table::activate();
    cpu.leave_process();

    println!(
        "[process] hello took {} timer interrupts in user mode and returned from each",
        process.user_ticks
    );
    process.status
}

/// Enters user mode on the process page table and kernel stack established by [`run`].
extern "C" fn enter(control_block: usize) -> ! {
    // SAFETY: `run` is suspended with this live `Process`, exclusively used by this context.
    let entry = unsafe { (*(control_block as *const Process)).entry };
    let frame = TrapFrame::for_user(entry, user_table::STACK_TOP);

    println!("[process] entering user mode at {entry:#x}, sp {:#x}", user_table::STACK_TOP);
    // SAFETY: the live process table maps the entry point and user stack with user permissions.
    unsafe { trap::resume(&frame) }
}

/// End the running process with `status`, and give the hart back to [`run`].
///
/// This function does not return; it abandons the active trap frame.
///
/// # Panics
///
/// Panics if no process is running on this hart.
pub(crate) fn exit(status: usize) -> ! {
    let process = current();

    // SAFETY: `run`'s `Process`, on a stack suspended inside the switch below, so this context is
    // the only one touching it. This frame and its stack are abandoned.
    unsafe {
        (*process).status = status;
        context::switch_to(&raw const (*process).resume)
    }
}

pub(crate) fn kill() -> ! { exit(FAULT_STATUS) }

pub(crate) fn record_user_tick() {
    // SAFETY: the active user trap has exclusive access to the running process.
    unsafe { (*current()).user_ticks += 1 };
}

fn current() -> *mut Process {
    cpu::current().process().expect("no process is running on this hart") as *mut Process
}

//! User processes: what one is loaded from, what it runs in, and what running one costs the
//! context that starts it.
//!
//! [`image`] reads an executable; [`crate::memory::user_table`] turns what it read into an address
//! space. [`run`] is the one step across them, and the whole of what running a process means while
//! there is one: the calling context blocks for as long as the program lives, and [`exit`] is what
//! hands the hart back to it.
//!
//! What a process *is* settles here — a kernel stack, an address space, a place to begin and a
//! context to return to. Choosing between two of them is a question nothing below asks: there is no
//! run queue, no time slice and no second process, so a switch happens exactly twice per program.

pub mod image;

use mmu::VirtualAddr;

use crate::arch::context::{self, KernelContext};
use crate::arch::interrupts;
use crate::arch::trap::{self, TrapFrame};
use crate::cpu;
use crate::memory::address_space::AddressSpace;
use crate::memory::{self, kernel_table, user_table};

/// The one user program this kernel runs, embedded in the image.
///
/// `CARGO_BIN_FILE_HELLO` comes from the artifact dependency in `Cargo.toml`, so cargo builds the
/// program before the kernel and names the file it produced. It moves to the disk once there is a
/// block driver and a filesystem to read it with.
static HELLO: &[u8] = include_bytes!(env!("CARGO_BIN_FILE_HELLO"));

/// The status a process gets when the kernel ends it rather than the program asking to end: the
/// shell's convention for a program killed by `SIGSEGV`.
const FAULT_STATUS: usize = 128 + 11;

/// A running process, and everything a trap taken inside it has to be able to find.
///
/// On [`run`]'s stack, because `run` blocks for exactly as long as the process lives: a `static`
/// would be a second answer to which process a hart is running, and the heap would keep the block
/// alive past the frame that owns it. [`crate::cpu`] carries the pointer, which is how the trap path
/// arrives back here.
struct Process {
    space: AddressSpace,
    /// Where the program begins, from its own program headers.
    entry: VirtualAddr,
    /// The context [`exit`] resumes: [`run`]'s own, suspended inside the switch that started the
    /// process.
    resume: KernelContext,
    /// Timer interrupts taken while this process was in user mode.
    ///
    /// The count is what makes the return half of the privilege switch observable: a process that
    /// reaches `exit` having taken some was interrupted and put back, where one that was merely
    /// entered reports none.
    user_ticks: u64,
    /// What the program asked to exit with, or [`FAULT_STATUS`].
    status: usize,
}

/// Load the embedded program, run it to completion, and answer with the status it exited on.
///
/// Blocks. The hart goes into user mode and comes back here when the process exits, on the kernel's
/// own page table with nothing of the process still running.
///
/// # Panics
///
/// If the embedded image is not one this kernel can run, which is a build that produced something
/// unexpected rather than a runtime condition, or if there is no memory for the process.
pub fn run() -> usize {
    // Before the address space, which copies the kernel's half as it stands: a stack mapped
    // afterwards would be invisible in the table the process runs on, and the first trap out of
    // user mode would have nowhere to push.
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

    // SAFETY: a space that shares the kernel's upper half, where this hart's PC and stack pointer
    // both live, so both keep their frames across the write.
    unsafe { process.space.activate() };

    // A context switch carries no CSR, so the hart arrives back from `exit` with interrupts as the
    // trap that called it left them — masked. Borrowing the bit across the switch is what puts it
    // back, and the context that owns it is the one asking.
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

/// A process's first entry into user mode: the kernel side of a trap that never happened.
///
/// Runs on the process's kernel stack, under the process's page table, both established by [`run`].
/// The frame it builds is abandoned along with the stack space it sits in — the next trap out of
/// user mode builds its own at the same place, which is what the stack top in
/// [`Cpu`](crate::cpu::Cpu) names.
extern "C" fn enter(control_block: usize) -> ! {
    // SAFETY: the `Process` `run` left on its own stack, which is suspended inside the switch that
    // reached here, so this context is the only one touching it.
    let entry = unsafe { (*(control_block as *const Process)).entry };
    let frame = TrapFrame::for_user(entry, user_table::STACK_TOP);

    println!("[process] entering user mode at {entry:#x}, sp {:#x}", user_table::STACK_TOP);
    // SAFETY: an entry point and a stack the live table maps for user mode, which
    // `user_table::build` installed and audited before this context existed.
    unsafe { trap::resume(&frame) }
}

/// End the running process with `status`, and give the hart back to [`run`].
///
/// Called from inside the trap that carried the request, on the process's kernel stack. Nothing
/// returns from here: the frame that trap would have resumed is abandoned with the stack it sits on,
/// and [`run`]'s context is resumed in its place.
///
/// # Panics
///
/// If no process is running on this hart, which means something other than a trap from user mode
/// reached here.
pub(crate) fn exit(status: usize) -> ! {
    let process = current();

    // SAFETY: `run`'s `Process`, on a stack suspended inside the switch below, so this context is
    // the only one touching it.
    unsafe {
        (*process).status = status;
        // Saved into a context on the process's kernel stack that nothing will resume: the switch
        // is one-way.
        let mut spent = KernelContext::default();
        context::switch(&raw mut spent, &raw const (*process).resume);
    }

    unreachable!("switched back into a process that had already exited")
}

/// End the running process because the kernel cannot resume it. See [`FAULT_STATUS`].
pub(crate) fn kill() -> ! { exit(FAULT_STATUS) }

/// Count a timer interrupt taken while the running process was in user mode.
pub(crate) fn record_user_tick() {
    // SAFETY: as `exit`'s — the trap this runs inside is one the process took.
    unsafe { (*current()).user_ticks += 1 };
}

/// The process running on this hart.
///
/// # Panics
///
/// If there is none, which means the trap path reached a process operation on a hart that never
/// entered user mode.
fn current() -> *mut Process {
    cpu::current().process().expect("no process is running on this hart") as *mut Process
}

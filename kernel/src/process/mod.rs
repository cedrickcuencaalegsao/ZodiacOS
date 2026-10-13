pub mod context;
pub mod programs;
pub mod syscall;
pub mod task;

use crate::base::error::Error;
use crate::base::sync::Mutex;
use crate::scheduler::{self, MAX_TASKS};
use crate::{arch, kassert, paging, pmm, vga};
use crate::{kprintln, slogln};
use task::{name_str, Entry, State, Task};

// Each kernel stack gets a slot in its own virtual region: one unmapped guard page
// (so an overflow faults instead of corrupting a neighbour) plus 32 KiB of stack.
const STACK_PAGES: u64 = 8;
const SLOT_BYTES: u64 = (STACK_PAGES + 1) * 4096;

// User programs live in the user part of an address space
const USER_CODE: u64 = paging::USER_BASE;
const USER_STACK: u64 = paging::USER_BASE + 0x10_0000;

/// Page-table edits for stacks are serialized with a (blocking) Mutex.
static STACK_LOCK: Mutex<()> = Mutex::new(());

fn stack_base(slot: usize) -> u64 {
    paging::STACK_REGION + slot as u64 * SLOT_BYTES
}

/// If `addr` lies in the guard page of some task's stack, which slot?
pub fn stack_guard_slot(addr: u64) -> Option<usize> {
    let end = paging::STACK_REGION + MAX_TASKS as u64 * SLOT_BYTES;
    if addr < paging::STACK_REGION || addr >= end {
        return None;
    }
    let offset = addr - paging::STACK_REGION;
    if offset % SLOT_BYTES < 4096 {
        Some((offset / SLOT_BYTES) as usize)
    } else {
        None
    }
}

fn unmap_stack_locked(slot: usize) {
    let pml4 = paging::kernel_pml4();
    let base = stack_base(slot);
    for page in 0..STACK_PAGES {
        if let Some(frame) = paging::unmap_page(pml4, base + (page + 1) * 4096) {
            pmm::free_frame(frame);
        }
    }
}

fn unmap_stack(slot: usize) {
    let _lock = STACK_LOCK.lock();
    unmap_stack_locked(slot);
}

/// Allocates and maps the stack pages (the guard page below stays unmapped).
/// Returns the stack top.
fn map_stack(slot: usize) -> Result<u64, Error> {
    let _lock = STACK_LOCK.lock();
    let pml4 = paging::kernel_pml4();
    let base = stack_base(slot);
    for page in 0..STACK_PAGES {
        let Some(frame) = pmm::alloc_frame() else {
            unmap_stack_locked(slot);
            return Err(Error::OutOfMemory);
        };
        if paging::map_page(pml4, base + (page + 1) * 4096, frame, paging::WRITABLE | paging::NX).is_err() {
            pmm::free_frame(frame);
            unmap_stack_locked(slot);
            return Err(Error::OutOfMemory);
        }
    }
    Ok(base + SLOT_BYTES)
}

/// Lays out a fresh stack so that the first context switch "returns" into `task_start`.
/// Layout, low to high: r15 r14 r13 r12 rbx rbp | return address | 0
fn init_stack(top: u64) -> u64 {
    let sp = top - 64;
    unsafe {
        let words = sp as *mut u64;
        for i in 0..6 {
            words.add(i).write(0);
        }
        words.add(6).write(task_start as *const () as u64);
        words.add(7).write(0);
    }
    sp
}

/// Where every new task begins. Interrupts are still off from the context switch.
extern "C" fn task_start() -> ! {
    arch::enable_interrupts();
    let entry = scheduler::with(|s| s.tasks[s.current].entry);
    match entry {
        Entry::Kernel(f) => f(),
        Entry::User { rip, rsp } => unsafe { context::zodiac_enter_user(rip, rsp) },
        Entry::None => {}
    }
    exit_current(0)
}

fn create(name: &str, entry: Entry, space: u64, enqueue: bool) -> Result<(usize, u32), Error> {
    // 1. reserve a slot (short critical section)
    let reserved = scheduler::with(|s| {
        let slot = (1..MAX_TASKS).find(|&i| s.tasks[i].state == State::Free)?;
        s.tasks[slot].state = State::Embryo;
        let pid = s.next_pid;
        s.next_pid += 1;
        Some((slot, pid))
    });
    let (slot, pid) = reserved.ok_or(Error::Busy)?;

    // 2. build the stack (allocates memory, so not under the scheduler lock)
    let top = match map_stack(slot) {
        Ok(top) => top,
        Err(e) => {
            scheduler::with(|s| s.tasks[slot] = Task::EMPTY);
            return Err(e);
        }
    };
    let rsp = init_stack(top);

    // 3. publish the task
    scheduler::with(|s| {
        let t = &mut s.tasks[slot];
        *t = Task {
            state: State::Ready,
            pid,
            entry,
            rsp,
            kstack_base: stack_base(slot),
            kstack_top: top,
            address_space: space,
            ..Task::EMPTY
        };
        t.set_name(name);
        if enqueue {
            s.queue.push(slot);
        }
    });
    Ok((slot, pid))
}

/// Starts a kernel thread. Returns its pid. Call `join` or `detach` on it later.
pub fn spawn(name: &str, entry: fn()) -> Result<u32, Error> {
    create(name, Entry::Kernel(entry), 0, true).map(|(_, pid)| pid)
}

/// Allocates a frame, maps it in `space` and returns its physical address.
fn map_user_page(space: u64, virt: u64, flags: u64) -> Result<u64, Error> {
    let frame = pmm::alloc_zeroed().ok_or(Error::OutOfMemory)?;
    if paging::map_page(space, virt, frame, flags).is_err() {
        pmm::free_frame(frame);
        return Err(Error::OutOfMemory);
    }
    Ok(frame)
}

/// Starts a user process: a new address space with `program` mapped read-only and
/// executable at the start of user space, plus one stack page. Returns its pid.
pub fn spawn_user(name: &str, program: &[u8]) -> Result<u32, Error> {
    kassert!(
        !program.is_empty() && program.len() <= 4096,
        "a user program must fit in one page"
    );
    let space = paging::new_address_space().ok_or(Error::OutOfMemory)?;

    let setup = (|| -> Result<(), Error> {
        // not writable: user code can't modify itself
        let code = map_user_page(space, USER_CODE, paging::USER)?;
        unsafe { core::ptr::copy_nonoverlapping(program.as_ptr(), code as *mut u8, program.len()) };
        map_user_page(space, USER_STACK, paging::USER | paging::WRITABLE | paging::NX)?;
        Ok(())
    })();
    if let Err(e) = setup {
        paging::destroy_address_space(space);
        return Err(e);
    }

    let entry = Entry::User { rip: USER_CODE, rsp: USER_STACK + 4096 };
    match create(name, entry, space, true) {
        Ok((_, pid)) => Ok(pid),
        Err(e) => {
            paging::destroy_address_space(space);
            Err(e)
        }
    }
}

/// Ends the running task. Its slot stays a zombie until joined or reaped.
pub fn exit_current(code: i32) -> ! {
    scheduler::with(|s| {
        let cur = s.current;
        kassert!(cur != 0, "the boot thread cannot exit");
        s.tasks[cur].exit_code = code;
        s.tasks[cur].state = State::Zombie;
    });
    scheduler::schedule();
    unreachable!("a finished task was scheduled again");
}

/// Frees a zombie's stack and address space. Returns its exit code.
fn reap_slot(slot: usize) -> i32 {
    let (space, code) = scheduler::with(|s| (s.tasks[slot].address_space, s.tasks[slot].exit_code));
    unmap_stack(slot);
    if space != 0 {
        paging::destroy_address_space(space);
    }
    scheduler::with(|s| s.tasks[slot] = Task::EMPTY);
    code
}

/// Waits for a task to finish and returns its exit code (None if no such task).
/// Polls every 10 ms rather than keeping per-task wait queues.
pub fn join(pid: u32) -> Option<i32> {
    loop {
        let found = scheduler::with(|s| {
            s.tasks
                .iter()
                .position(|t| t.pid == pid && t.state != State::Free)
                .map(|i| (i, s.tasks[i].state))
        });
        match found {
            None => return None,
            Some((slot, State::Zombie)) => return Some(reap_slot(slot)),
            Some(_) => scheduler::sleep_ms(10),
        }
    }
}

/// Nobody will join this task: let the idle task clean it up when it finishes.
#[allow(dead_code)]
pub fn detach(pid: u32) {
    scheduler::with(|s| {
        if let Some(t) = s.tasks.iter_mut().find(|t| t.pid == pid && t.state != State::Free) {
            t.detached = true;
        }
    });
}

pub fn reap_zombies() {
    for slot in 1..MAX_TASKS {
        let reap = scheduler::with(|s| {
            s.tasks[slot].state == State::Zombie && s.tasks[slot].detached && slot != s.current
        });
        if reap {
            reap_slot(slot);
        }
    }
}

/// Called from the exception handlers when code running in user mode faults.
/// The process dies; the kernel keeps running.
pub fn user_fault(title: &str, rip: u64, code: Option<u64>, cr2: Option<u64>) -> ! {
    let (pid, name) = scheduler::current_info();
    vga::set_color(0x0C);
    match cr2 {
        Some(addr) => kprintln!(
            "\n[process {} '{}' killed: {} at {:#x}, address {:#x}]",
            pid, name_str(&name), title, rip, addr
        ),
        None => kprintln!(
            "\n[process {} '{}' killed: {} at {:#x}]",
            pid, name_str(&name), title, rip
        ),
    }
    vga::set_color(0x0F);
    slogln!("[PROCESS] pid {} killed by {} at {:#x}", pid, title, rip);
    let _ = code;
    exit_current(-11)
}

fn idle_main() {
    loop {
        reap_zombies();
        unsafe { core::arch::asm!("hlt") }; // sleep until the next interrupt
    }
}

/// Creates the idle task and opens the system call gate.
pub fn init() {
    syscall::init();
    let (slot, _) = create("idle", Entry::Kernel(idle_main), 0, false)
        .expect("process: cannot create the idle task");
    scheduler::set_idle(slot);
    crate::log_info!("scheduler: round-robin, {} ms slice, idle task in slot {}",
        scheduler::policy::QUANTUM_TICKS as u64 * 1000 / crate::interrupts::TIMER_HZ as u64, slot);
}
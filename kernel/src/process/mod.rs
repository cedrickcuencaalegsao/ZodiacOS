#![allow(dead_code)]
pub mod context;
pub mod programs;
pub mod syscall;
pub mod task;

use crate::base::sync::Mutex;
use crate::scheduler::{self, MAX_TASKS};
use crate::{arch, kassert, paging, pmm, vga};
use crate::{kprintln, slogln};
use errors::Error;
use task::{Entry, State, Task, name_str};

// Each kernel stack gets a slot in its own virtual region: one unmapped guard page
// (so an overflow faults instead of corrupting a neighbour) plus 32 KiB of stack.
const STACK_PAGES: u64 = 8;
const SLOT_BYTES: u64 = (STACK_PAGES + 1) * 4096;

// User programs live in the user part of an address space.
// The code region is now 256 MiB wide so a real ELF (desktop) fits below the stack.
const USER_CODE: u64 = paging::USER_BASE;
const USER_STACK: u64 = paging::USER_BASE + 0x1000_0000;
const USER_STACK_PAGES: u64 = 16; // 64 KiB user stack for ELF programs

// ELF constants
const PT_LOAD: u32 = 1;
const PF_X: u32 = 1;
const PF_W: u32 = 2;

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
        if paging::map_page(
            pml4,
            base + (page + 1) * 4096,
            frame,
            paging::WRITABLE | paging::NX,
        )
        .is_err()
        {
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

/// Starts a user process from a flat binary: a new address space with `program` mapped
/// read-only and executable at the start of user space, plus one stack page.
/// Kept for the tiny built-in programs in `programs.rs`. Returns its pid.
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
        map_user_page(
            space,
            USER_STACK,
            paging::USER | paging::WRITABLE | paging::NX,
        )?;
        Ok(())
    })();
    if let Err(e) = setup {
        paging::destroy_address_space(space);
        return Err(e);
    }

    let entry = Entry::User {
        rip: USER_CODE,
        rsp: USER_STACK + 4096,
    };
    match create(name, entry, space, true) {
        Ok((_, pid)) => Ok(pid),
        Err(e) => {
            paging::destroy_address_space(space);
            Err(e)
        }
    }
}

// ---------------------------------------------------------------------------
// ELF loading
// ---------------------------------------------------------------------------

fn rd16(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        b.get(o..o.checked_add(2)?)?.try_into().ok()?,
    ))
}
fn rd32(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        b.get(o..o.checked_add(4)?)?.try_into().ok()?,
    ))
}
fn rd64(b: &[u8], o: usize) -> Option<u64> {
    Some(u64::from_le_bytes(
        b.get(o..o.checked_add(8)?)?.try_into().ok()?,
    ))
}

/// Maps every PT_LOAD segment of `image` into `space`, plus the user stack.
/// Returns the entry point.
///
/// Requirements on the program: statically linked, non-PIE ELF64 for x86_64, linked at or
/// above `paging::USER_BASE`, with every segment starting on its own 4 KiB page
/// (use `ALIGN(4096)` between sections in the linker script).
fn load_elf(space: u64, image: &[u8]) -> Result<u64, Error> {
    let bad = || Error::InvalidExecutable; // add this variant to base::error::Error

    // magic, ELFCLASS64, little-endian, machine = x86_64
    if image.get(0..4) != Some(&[0x7F, b'E', b'L', b'F'][..])
        || image.get(4) != Some(&2)
        || image.get(5) != Some(&1)
        || rd16(image, 18) != Some(0x3E)
    {
        return Err(bad());
    }
    let entry = rd64(image, 24).ok_or_else(bad)?;
    let phoff = rd64(image, 32).ok_or_else(bad)? as usize;
    let phentsize = rd16(image, 54).ok_or_else(bad)? as usize;
    let phnum = rd16(image, 56).ok_or_else(bad)? as usize;

    let stack_bottom = USER_STACK + 4096 - USER_STACK_PAGES * 4096;

    for i in 0..phnum {
        let ph = phoff
            .checked_add(i.checked_mul(phentsize).ok_or_else(bad)?)
            .ok_or_else(bad)?;
        if rd32(image, ph) != Some(PT_LOAD) {
            continue;
        }
        let flags = rd32(image, ph + 4).ok_or_else(bad)?;
        let offset = rd64(image, ph + 8).ok_or_else(bad)? as usize;
        let vaddr = rd64(image, ph + 16).ok_or_else(bad)?;
        let filesz = rd64(image, ph + 32).ok_or_else(bad)? as usize;
        let memsz = rd64(image, ph + 40).ok_or_else(bad)?;

        // never let a program map kernel memory or collide with its own stack
        let end = vaddr.checked_add(memsz).ok_or_else(bad)?;
        if vaddr < USER_CODE || end > stack_bottom || filesz as u64 > memsz {
            return Err(bad());
        }
        let data = image
            .get(offset..offset.checked_add(filesz).ok_or_else(bad)?)
            .ok_or_else(bad)?;

        let mut pflags = paging::USER;
        if flags & PF_W != 0 {
            pflags |= paging::WRITABLE;
        }
        if flags & PF_X == 0 {
            pflags |= paging::NX;
        }

        let mut page = vaddr & !0xFFF;
        while page < end {
            let frame = map_user_page(space, page, pflags)?; // zeroed, so .bss comes for free
            // copy the part of the file data that lands in this page
            let lo = core::cmp::max(page, vaddr);
            let hi = core::cmp::min(page + 4096, vaddr + filesz as u64);
            if lo < hi {
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        data.as_ptr().add((lo - vaddr) as usize),
                        (frame + (lo - page)) as *mut u8,
                        (hi - lo) as usize,
                    );
                }
            }
            page += 4096;
        }
    }

    for p in 0..USER_STACK_PAGES {
        map_user_page(
            space,
            stack_bottom + p * 4096,
            paging::USER | paging::WRITABLE | paging::NX,
        )?;
    }
    Ok(entry)
}

/// Starts a user process from an ELF image (e.g. the desktop). Returns its pid.
pub fn spawn_elf(name: &str, image: &[u8]) -> Result<u32, Error> {
    let space = paging::new_address_space().ok_or(Error::OutOfMemory)?;
    let rip = match load_elf(space, image) {
        Ok(rip) => rip,
        Err(e) => {
            paging::destroy_address_space(space);
            return Err(e);
        }
    };
    let entry = Entry::User {
        rip,
        rsp: USER_STACK + 4096,
    };
    match create(name, entry, space, true) {
        Ok((_, pid)) => Ok(pid),
        Err(e) => {
            paging::destroy_address_space(space);
            Err(e)
        }
    }
}

// ---------------------------------------------------------------------------
// Task lifecycle
// ---------------------------------------------------------------------------

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
    let (space, code, pid) = scheduler::with(|s| {
        (
            s.tasks[slot].address_space,
            s.tasks[slot].exit_code,
            s.tasks[slot].pid,
        )
    });
    unmap_stack(slot);
    if space != 0 {
        paging::destroy_address_space(space);
    }
    crate::filesystem::file::process_exit(pid);
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
        if let Some(t) = s
            .tasks
            .iter_mut()
            .find(|t| t.pid == pid && t.state != State::Free)
        {
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
            pid,
            name_str(&name),
            title,
            rip,
            addr
        ),
        None => kprintln!(
            "\n[process {} '{}' killed: {} at {:#x}]",
            pid,
            name_str(&name),
            title,
            rip
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
    crate::log_info!(
        "scheduler: round-robin, {} ms slice, idle task in slot {}",
        scheduler::policy::QUANTUM_TICKS as u64 * 1000 / crate::interrupts::TIMER_HZ as u64,
        slot
    );
}

pub mod policy;
pub mod run_queue;

use crate::base::sync::IrqSpinLock;
use crate::kassert;
use crate::process::context::zodiac_switch;
use crate::process::task::{Entry, State, Task};
use crate::{arch, interrupts, paging};
use run_queue::RunQueue;

pub const MAX_TASKS: usize = 32;

/// The task table and run queue. Only reachable through `with()` or `schedule()`,
/// both of which keep interrupts off while the lock is held. Nothing in here may
/// allocate: the timer interrupt runs this code.
pub(crate) struct Sched {
    pub tasks: [Task; MAX_TASKS],
    pub queue: RunQueue,
    pub current: usize,
    pub idle: usize,
    pub next_pid: u32,
    pub ready: bool,
}

static SCHED: IrqSpinLock<Sched> = IrqSpinLock::new(Sched {
    tasks: [Task::EMPTY; MAX_TASKS],
    queue: RunQueue::new(),
    current: 0,
    idle: 0,
    next_pid: 1,
    ready: false,
});

pub(crate) fn with<R>(f: impl FnOnce(&mut Sched) -> R) -> R {
    let mut guard = SCHED.lock();
    f(&mut guard)
}

struct Switch {
    old_rsp: *mut u64,
    new_rsp: u64,
    cr3: u64,
    kstack_top: u64,
}

impl Sched {
    /// Round robin: put the running task at the back of the queue, take the front.
    fn pick_next(&mut self) -> Option<Switch> {
        let cur = self.current;

        if self.tasks[cur].state == State::Running && cur != self.idle {
            self.tasks[cur].state = State::Ready;
            self.queue.push(cur);
        }

        // nothing runnable: run the idle task
        let next = self.queue.pop().unwrap_or(self.idle);

        if next == cur {
            self.tasks[cur].state = State::Running;
            self.tasks[cur].slice_left = policy::new_slice();
            return None;
        }

        if cur == self.idle && self.tasks[cur].state == State::Running {
            self.tasks[cur].state = State::Ready; // idle is never queued
        }
        self.tasks[next].state = State::Running;
        self.tasks[next].slice_left = policy::new_slice();
        self.current = next;

        let (new_rsp, space, kstack_top) = {
            let t = &self.tasks[next];
            (t.rsp, t.address_space, t.kstack_top)
        };
        Some(Switch {
            old_rsp: &mut self.tasks[cur].rsp as *mut u64,
            new_rsp,
            cr3: if space == 0 { paging::kernel_pml4() } else { space },
            kstack_top,
        })
    }

    /// Per-tick bookkeeping. Returns true when the running task should give up the CPU.
    fn on_tick(&mut self, now: u64) -> bool {
        for i in 0..MAX_TASKS {
            if self.tasks[i].state == State::Sleeping && self.tasks[i].wake_tick <= now {
                self.tasks[i].state = State::Ready;
                self.queue.push(i);
            }
        }

        let cur = self.current;
        if cur == self.idle {
            return !self.queue.is_empty();
        }
        let t = &mut self.tasks[cur];
        t.cpu_ticks += 1;
        if t.state != State::Running {
            return true; // blocked or finished but still on the CPU: move on
        }
        policy::slice_expired(&mut t.slice_left) && !self.queue.is_empty()
    }
}

/// Registers the boot thread (the one running `kernel_main`) as task 0.
pub fn init() {
    with(|s| {
        let boot = &mut s.tasks[0];
        *boot = Task::EMPTY;
        boot.state = State::Running;
        boot.set_name("kernel");
        boot.kstack_top = 0x90000; // the boot stack
        boot.slice_left = policy::new_slice();
        s.current = 0;
        s.ready = true;
    });
    interrupts::set_tick_hook(tick);
}

pub fn ready() -> bool {
    with(|s| s.ready)
}

pub fn set_idle(slot: usize) {
    with(|s| s.idle = slot);
}

/// Switches to the next runnable task (or returns at once if it is the same one).
/// Safe to call from a thread or from the timer interrupt.
pub fn schedule() {
    let was_enabled = arch::save_and_disable_interrupts();

    // The lock is released before the switch: the next task must be able to take it.
    let switch = SCHED.raw().lock().pick_next();

    if let Some(sw) = switch {
        arch::current::gdt::set_kernel_stack(sw.kstack_top);
        if paging::current() != sw.cr3 {
            paging::activate(sw.cr3);
        }
        unsafe { zodiac_switch(sw.old_rsp, sw.new_rsp) };
        // we only get here again when some later schedule() switches back to us
    }

    arch::restore_interrupts(was_enabled);
}

pub fn yield_now() {
    schedule();
}

/// Called from the timer interrupt (100 times a second, interrupts off).
pub fn tick(now: u64) {
    let reschedule = SCHED.raw().lock().on_tick(now);
    if reschedule {
        schedule();
    }
}

pub fn sleep_ms(ms: u64) {
    kassert!(arch::interrupts_enabled(), "sleep_ms with interrupts disabled would hang");
    let ticks = ((ms * interrupts::TIMER_HZ as u64 + 999) / 1000).max(1);
    with(|s| {
        let cur = s.current;
        s.tasks[cur].wake_tick = interrupts::ticks() + ticks;
        s.tasks[cur].state = State::Sleeping;
    });
    schedule();
}

pub fn current_slot() -> usize {
    with(|s| s.current)
}

pub fn current_pid() -> u32 {
    with(|s| s.tasks[s.current].pid)
}

pub fn current_info() -> (u32, [u8; 16]) {
    with(|s| (s.tasks[s.current].pid, s.tasks[s.current].name))
}

/// Used by the double-fault handler: never waits for the lock.
pub fn name_of(slot: usize) -> Option<[u8; 16]> {
    if slot >= MAX_TASKS {
        return None;
    }
    let guard = SCHED.raw().try_lock()?;
    Some(guard.tasks[slot].name)
}

/// Marks the running task blocked. The caller must call `schedule()` afterwards.
pub fn block_current() {
    with(|s| {
        let cur = s.current;
        s.tasks[cur].state = State::Blocked;
    });
}

pub fn wake_slot(slot: usize) {
    with(|s| {
        if matches!(s.tasks[slot].state, State::Blocked | State::Sleeping) {
            s.tasks[slot].state = State::Ready;
            s.queue.push(slot);
        }
    });
}

#[derive(Clone, Copy)]
pub struct TaskInfo {
    pub pid: u32,
    pub name: [u8; 16],
    pub state: State,
    pub cpu_ticks: u64,
    pub user: bool,
}

/// A copy of the task table for `ps`. Built into a fixed array: no allocating under the lock.
pub fn snapshot() -> [Option<TaskInfo>; MAX_TASKS] {
    with(|s| {
        let mut out = [None; MAX_TASKS];
        for (i, t) in s.tasks.iter().enumerate() {
            if t.state != State::Free {
                out[i] = Some(TaskInfo {
                    pid: t.pid,
                    name: t.name,
                    state: t.state,
                    cpu_ticks: t.cpu_ticks,
                    user: matches!(t.entry, Entry::User { .. }),
                });
            }
        }
        out
    })
}
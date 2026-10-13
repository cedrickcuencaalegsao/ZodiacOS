use crate::base::sync::Mutex;
use crate::process::{self, programs, task};
use crate::{interrupts, scheduler};
use crate::kprintln;
use core::sync::atomic::{AtomicUsize, Ordering::Relaxed};

pub fn run(cmd: &str, args: &[&str]) {
    match cmd {
        "ps" => ps(),
        "threads" => threads(),
        "syncdemo" => syncdemo(),
        "userdemo" => userdemo(args),
        _ => {}
    }
}

fn ps() {
    kprintln!("{:<4} {:<12} {:<9} {:>8}  {}", "PID", "NAME", "STATE", "CPU(ms)", "KIND");
    for info in scheduler::snapshot().iter().flatten() {
        kprintln!(
            "{:<4} {:<12} {:<9} {:>8}  {}",
            info.pid,
            task::name_str(&info.name),
            info.state.name(),
            info.cpu_ticks * 1000 / interrupts::TIMER_HZ as u64,
            if info.user { "user" } else { "kernel" }
        );
    }
}

// ---- Preemption demo: threads that never yield still take turns ----------------
fn worker() {
    let pid = scheduler::current_pid();
    for step in 1..=5 {
        kprintln!("[thread {}] step {}", pid, step);
        // burn about 80 ms of CPU: longer than a 50 ms slice, so the timer preempts us
        let start = interrupts::ticks();
        while interrupts::ticks() - start < 8 {
            core::hint::spin_loop();
        }
    }
}

fn threads() {
    let mut pids = [0u32; 3];
    for pid in pids.iter_mut() {
        match process::spawn("worker", worker) {
            Ok(p) => *pid = p,
            Err(e) => {
                kprintln!("threads: {}", e);
                return;
            }
        }
    }
    for pid in pids {
        process::join(pid);
    }
    kprintln!("all threads finished");
}

// ---- Synchronization demo ---------------------------------------------------------
const THREADS: usize = 4;
const ROUNDS: usize = 1000;

static RACY: AtomicUsize = AtomicUsize::new(0);
static ATOMIC: AtomicUsize = AtomicUsize::new(0);
static LOCKED: Mutex<usize> = Mutex::new(0);

fn sync_worker() {
    for _ in 0..ROUNDS {
        // 1. read, give up the CPU, write back: other threads' updates get lost
        let seen = RACY.load(Relaxed);
        scheduler::yield_now();
        RACY.store(seen + 1, Relaxed);

        // 2. one atomic read-modify-write: nothing is lost
        ATOMIC.fetch_add(1, Relaxed);

        // 3. a plain integer behind a blocking Mutex
        *LOCKED.lock() += 1;
    }
}

fn syncdemo() {
    RACY.store(0, Relaxed);
    ATOMIC.store(0, Relaxed);
    *LOCKED.lock() = 0;

    let mut pids = [0u32; THREADS];
    for pid in pids.iter_mut() {
        match process::spawn("sync-worker", sync_worker) {
            Ok(p) => *pid = p,
            Err(e) => {
                kprintln!("syncdemo: {}", e);
                return;
            }
        }
    }
    for pid in pids {
        process::join(pid);
    }

    kprintln!("{} threads x {} rounds, expected {}", THREADS, ROUNDS, THREADS * ROUNDS);
    kprintln!("unprotected counter: {}   (updates lost)", RACY.load(Relaxed));
    kprintln!("atomic counter:      {}", ATOMIC.load(Relaxed));
    kprintln!("mutex counter:       {}", *LOCKED.lock());
}

// ---- User mode and system calls ---------------------------------------------------
fn userdemo(args: &[&str]) {
    let Some(&name) = args.first() else {
        kprintln!("usage: userdemo <hello|read|kptr|priv>");
        return;
    };
    let Some(program) = programs::get(name) else {
        kprintln!("userdemo: unknown program '{}'", name);
        return;
    };
    match process::spawn_user(name, program) {
        Ok(pid) => {
            kprintln!("started user process {} ({})", pid, name);
            if let Some(code) = process::join(pid) {
                kprintln!("process {} exited with code {}", pid, code);
            }
        }
        Err(e) => kprintln!("userdemo: {}", e),
    }
}

// ---- Kernel stack overflow: caught by the guard page --------------------------------
#[allow(unconditional_recursion)]
fn recurse(depth: u64) -> u64 {
    let buf = [depth as u8; 512];
    core::hint::black_box(&buf);
    recurse(depth + 1) + buf[0] as u64
}

fn overflow_thread() {
    recurse(0);
}

pub fn overflow_test() {
    match process::spawn("overflow", overflow_thread) {
        Ok(pid) => {
            process::join(pid);
        }
        Err(e) => kprintln!("crash stack: {}", e),
    }
}
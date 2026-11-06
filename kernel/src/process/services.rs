use crate::process::task::State as TaskState;
use crate::scheduler::{self, MAX_TASKS};
use alloc::{format, string::String, vec::Vec};
use core::sync::atomic::{AtomicU32, Ordering};
use spin::Mutex;

pub type Entry = fn();

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    Stopped,
    Running,
}

#[derive(Debug)]
pub enum SvcError {
    Exists,
    NotFound,
    AlreadyRunning,
    NotRunning,
    NoTemplate,
    InvalidName,
    Spawn,
}

struct Service {
    name: String,
    entry: Entry,
    state: State,
    pid: Option<u32>,
    restarts: u32,
}

pub struct Info {
    pub name: String,
    pub state: State,
    pub pid: Option<u32>,
    pub restarts: u32,
}

// Lock order: REGISTRY, then the scheduler lock (via scheduler::with).
// The scheduler never takes REGISTRY.
static REGISTRY: Mutex<Vec<Service>> = Mutex::new(Vec::new());

/// Bit N set = the task in scheduler slot N has been asked to stop.
static STOP_MASK: AtomicU32 = AtomicU32::new(0);

// ---------- for service code ----------

/// Services call this in their main loop.
pub fn should_stop() -> bool {
    let slot = scheduler::current_slot();
    STOP_MASK.load(Ordering::Relaxed) & (1u32 << slot) != 0
}

/// Services call this when they are done. Never returns.
pub fn finish() -> ! {
    // ADAPT: if process/ has an exit function (e.g. `crate::process::exit()`), call it here.
    // Fallback: park the task forever. It no longer runs, but its slot is not freed.
    scheduler::block_current();
    loop {
        scheduler::schedule();
    }
}

// ---------- built-ins (read-only status entries, not tasks) ----------

pub struct Builtin {
    pub name: &'static str,
    pub status: fn() -> String,
    pub pid: fn() -> Option<u32>,
}

pub const BUILTINS: &[Builtin] = &[
    Builtin { name: "SessionManager", status: sessions_status, pid: sessions_pid },
    Builtin { name: "AuthManager", status: auth_status, pid: auth_pid },
    Builtin { name: "EthernetManager", status: ethernet_status, pid: ethernet_pid },
];


fn sessions_status() -> String {
    use crate::security::service as sec;
    match sec::whoami() {
        Some(s) if sec::is_elevated() => format!("active: {} (su)", s.name),
        Some(s) => format!("active: {}", s.name),
        None => "no active session".into(),
    }
}

fn auth_status() -> String {
    format!("{} account(s)", crate::security::service::list_users().len())
}

pub fn is_builtin(name: &str) -> bool {
    BUILTINS.iter().any(|b| b.name.eq_ignore_ascii_case(name))
}

// ---------- built-in templates for `service new` ----------

fn heartbeat() {
    while !should_stop() {
        crate::log_info!("heartbeat");
        crate::interrupts::sleep_ms(5000);
    }
    finish();
}

fn counter() {
    let mut n: u64 = 0;
    while !should_stop() {
        n += 1;
        crate::log_info!("counter: {}", n);
        crate::interrupts::sleep_ms(1000);
    }
    finish();
}

pub const TEMPLATES: &[(&str, Entry)] = &[("heartbeat", heartbeat), ("counter", counter)];

// ---------- the one function that depends on process/mod.rs ----------

/// ADAPT: match the real signature of `crate::process::spawn`.
/// Assumed: spawn(name: &str, entry: fn()) -> Result<u32 /* pid */, _>
fn spawn_task(name: &str, entry: Entry) -> Result<u32, SvcError> {
    crate::process::spawn(name, entry)
        .map(|pid| pid as u32)
        .map_err(|_| SvcError::Spawn)
}

// ---------- registry ----------

fn valid_name(n: &str) -> bool {
    !n.is_empty()
        && n.len() <= 15 // task names are [u8; 16]
        && !is_builtin(n)
        && n.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn slot_of(pid: u32) -> Option<usize> {
    scheduler::with(|s| {
        s.tasks
            .iter()
            .position(|t| t.state != TaskState::Free && t.pid == pid)
    })
}

/// Mark services whose task no longer exists (finished or crashed) as stopped.
fn refresh(r: &mut Vec<Service>) {
    for s in r.iter_mut() {
        if let Some(pid) = s.pid {
            if slot_of(pid).is_none() {
                s.pid = None;
                s.state = State::Stopped;
            }
        }
    }
}

pub fn register(name: &str, entry: Entry) -> Result<(), SvcError> {
    if !valid_name(name) {
        return Err(SvcError::InvalidName);
    }
    let mut r = REGISTRY.lock();
    if r.iter().any(|s| s.name == name) {
        return Err(SvcError::Exists);
    }
    r.push(Service {
        name: name.into(),
        entry,
        state: State::Stopped,
        pid: None,
        restarts: 0,
    });
    Ok(())
}

pub fn new_from_template(name: &str, template: &str) -> Result<(), SvcError> {
    let entry = TEMPLATES
        .iter()
        .find(|(t, _)| *t == template)
        .ok_or(SvcError::NoTemplate)?
        .1;
    register(name, entry)
}

pub fn list() -> Vec<Info> {
    let mut r = REGISTRY.lock();
    refresh(&mut r);
    r.iter()
        .map(|s| Info {
            name: s.name.clone(),
            state: s.state,
            pid: s.pid,
            restarts: s.restarts,
        })
        .collect()
}

pub fn start(name: &str) -> Result<(), SvcError> {
    let mut r = REGISTRY.lock();
    refresh(&mut r);
    let s = r.iter_mut().find(|s| s.name == name).ok_or(SvcError::NotFound)?;
    if s.state == State::Running {
        return Err(SvcError::AlreadyRunning);
    }
    // Clear stop bits of every free slot first: the new task will land in one of
    // them, so it can't inherit an old "stop" request.
    scheduler::with(|sc| {
        for i in 0..MAX_TASKS {
            if sc.tasks[i].state == TaskState::Free {
                STOP_MASK.fetch_and(!(1u32 << i), Ordering::Relaxed);
            }
        }
    });
    let pid = spawn_task(name, s.entry)?;
    s.pid = Some(pid);
    s.state = State::Running;
    Ok(())
}

/// Cooperative: sets the stop flag; the service exits when it next checks it.
pub fn kill(name: &str) -> Result<(), SvcError> {
    let mut r = REGISTRY.lock();
    refresh(&mut r);
    let s = r.iter_mut().find(|s| s.name == name).ok_or(SvcError::NotFound)?;
    let pid = s.pid.ok_or(SvcError::NotRunning)?;
    if let Some(slot) = slot_of(pid) {
        STOP_MASK.fetch_or(1u32 << slot, Ordering::Relaxed);
    }
    s.pid = None;
    s.state = State::Stopped;
    Ok(())
}

pub fn restart(name: &str) -> Result<(), SvcError> {
    match kill(name) {
        Ok(()) | Err(SvcError::NotRunning) => {}
        Err(e) => return Err(e),
    }
    // Let the old task see its flag and exit before the new one starts.
    crate::interrupts::sleep_ms(100);
    start(name)?;
    if let Some(s) = REGISTRY.lock().iter_mut().find(|s| s.name == name) {
        s.restarts += 1;
    }
    Ok(())
}

pub fn remove(name: &str) -> Result<(), SvcError> {
    let mut r = REGISTRY.lock();
    refresh(&mut r);
    let i = r.iter().position(|s| s.name == name).ok_or(SvcError::NotFound)?;
    if r[i].state == State::Running {
        return Err(SvcError::AlreadyRunning); // kill it first
    }
    r.remove(i);
    Ok(())
}
/// PID of a scheduler task by name (names are NUL-padded [u8; 16]).
fn pid_of_task(name: &str) -> Option<u32> {
    scheduler::snapshot()
        .iter()
        .flatten()
        .find(|t| t.name.iter().take_while(|&&c| c != 0).eq(name.as_bytes().iter()))
        .map(|t| t.pid)
}

fn ethernet_pid() -> Option<u32> {
    pid_of_task("net")
}

fn ethernet_status() -> String {
    use crate::drivers::network as net;

    let Some(dev) = net::interface() else {
        return String::from("no network interface");
    };
    let m = dev.mac();
    let cfg = net::config();
    let prefix: u32 = cfg.netmask.iter().map(|b| b.count_ones()).sum();
    let s = dev.stats();
    let thread = if ethernet_pid().is_some() { "" } else { " | net thread NOT running" };

    format!(
        "{} | link {} | {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} | {}.{}.{}.{}/{} gw {}.{}.{}.{} | rx {} tx {} pkts{}",
        dev.name(),
        if dev.link_up() { "up" } else { "down" },
        m[0], m[1], m[2], m[3], m[4], m[5],
        cfg.ip[0], cfg.ip[1], cfg.ip[2], cfg.ip[3],
        prefix,
        cfg.gateway[0], cfg.gateway[1], cfg.gateway[2], cfg.gateway[3],
        s.rx_packets, s.tx_packets,
        thread
    )
}

// ---------- manager daemons (own a PID; the data lives in security::service) ----------

fn sessiond() {
    crate::process::detach(scheduler::current_pid()); // same as the net thread
    loop {
        // placeholder for future housekeeping (idle-session expiry etc.)
        crate::interrupts::sleep_ms(1000);
    }
}

fn authd() {
    crate::process::detach(scheduler::current_pid());
    loop {
        // placeholder (failed-login lockout timers etc.)
        crate::interrupts::sleep_ms(1000);
    }
}

/// Call once at boot, after process::init() and interrupts::enable().
pub fn start_managers() {
    for (name, entry) in [("sessiond", sessiond as Entry), ("authd", authd as Entry)] {
        if let Err(e) = spawn_task(name, entry) {
            crate::log_warn!("cannot start {}: {:?}", name, e);
        }
    }
}

fn sessions_pid() -> Option<u32> {
    pid_of_task("sessiond")
}

fn auth_pid() -> Option<u32> {
    pid_of_task("authd")
}
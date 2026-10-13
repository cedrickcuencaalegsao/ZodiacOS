#[derive(Clone, Copy, PartialEq, Eq)]
pub enum State {
    Free,
    Embryo, // slot reserved, still being set up
    Ready,
    Running,
    Blocked,
    Sleeping,
    Zombie, // finished, waiting to be joined or reaped
}

impl State {
    pub fn name(self) -> &'static str {
        match self {
            State::Free => "free",
            State::Embryo => "embryo",
            State::Ready => "ready",
            State::Running => "running",
            State::Blocked => "blocked",
            State::Sleeping => "sleeping",
            State::Zombie => "zombie",
        }
    }
}

/// What a new task does when it first runs.
#[derive(Clone, Copy)]
pub enum Entry {
    None,
    Kernel(fn()),
    User { rip: u64, rsp: u64 },
}

#[derive(Clone, Copy)]
pub struct Task {
    pub state: State,
    pub pid: u32,
    pub name: [u8; 16],
    pub entry: Entry,
    pub rsp: u64,           // saved kernel stack pointer while not running
    pub kstack_base: u64,   // address of the guard page (0 for the boot stack)
    pub kstack_top: u64,
    pub address_space: u64, // PML4 physical address, 0 = the kernel's own
    pub wake_tick: u64,
    pub slice_left: u32,
    pub cpu_ticks: u64,
    pub exit_code: i32,
    pub detached: bool,     // reaped automatically by the idle task
}

impl Task {
    pub const EMPTY: Task = Task {
        state: State::Free,
        pid: 0,
        name: [0; 16],
        entry: Entry::None,
        rsp: 0,
        kstack_base: 0,
        kstack_top: 0,
        address_space: 0,
        wake_tick: 0,
        slice_left: 0,
        cpu_ticks: 0,
        exit_code: 0,
        detached: false,
    };

    pub fn set_name(&mut self, name: &str) {
        self.name = [0; 16];
        for (i, b) in name.bytes().take(15).enumerate() {
            self.name[i] = b;
        }
    }
}

pub fn name_str(name: &[u8; 16]) -> &str {
    let len = name.iter().position(|&b| b == 0).unwrap_or(16);
    core::str::from_utf8(&name[..len]).unwrap_or("?")
}
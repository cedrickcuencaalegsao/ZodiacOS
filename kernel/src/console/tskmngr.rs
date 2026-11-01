//! tskmngr - ZodiacOS task manager, btop-style.
//! Drop in: kernel/src/console/tskmngr.rs
//!
//! Layout (80x25 shown; scales with the terminal, minimum 60x20):
//!
//!   ┌─┤cpu├──────────────────────────────────────────────┤tskmngr├─┐
//!   │ area graph of total CPU │ brand / total meter / per-core     │
//!   └──────────────────────────────────────────┤up 00:01:23├───────┘
//!   ┌─┤mem├────────────────┐┌─┤proc├──────────────────┤12 tasks├───┐
//!   │ RAM  ■■■■■■■■■■■  42% ││  PID NAME     ST   MEM    CPU%      │
//!   ├─┤gpu├ ...            ││  ... selectable list + scrollbar     │
//!   ├─┤net├ rx/tx graphs   ││                                      │
//!
//! no_std, no alloc. Talks to the kernel only through `Platform` and `Term`.
//! Output uses CP437 glyphs (box drawing, blocks, arrows), which is what VGA
//! text mode renders natively. A framebuffer console must map CP437 bytes.
//!
//! Stack note: two snapshots + UI state live on the kernel stack (~16 KiB).
//! Run the shell on a stack of at least 32 KiB.
#![allow(dead_code)]

use core::fmt::{self, Display, Write};

pub const MAX_PROCS: usize = 64;
pub const MAX_NICS: usize = 4;
pub const MAX_CPUS: usize = 16;
/// History length (samples, one per second). Also the max graph width.
const HIST: usize = 64;

// VGA 16-colour palette indices
pub const BLACK: u8 = 0;
pub const BLUE: u8 = 1;
pub const DGREEN: u8 = 2;
pub const CYAN: u8 = 3;
pub const DRED: u8 = 4;
pub const MAGENTA: u8 = 5;
pub const BROWN: u8 = 6;
pub const GRAY: u8 = 7;
pub const DARK: u8 = 8;
pub const GREEN: u8 = 10;
pub const LCYAN: u8 = 11;
pub const RED: u8 = 12;
pub const YELLOW: u8 = 14;
pub const WHITE: u8 = 15;

// CP437 glyphs
const G_H: u8 = 0xC4; // ─
const G_V: u8 = 0xB3; // │
const G_TL: u8 = 0xDA; // ┌
const G_TR: u8 = 0xBF; // ┐
const G_BL: u8 = 0xC0; // └
const G_BR: u8 = 0xD9; // ┘
const G_RT: u8 = 0xB4; // ┤
const G_LT: u8 = 0xC3; // ├
const G_FULL: u8 = 0xDB; // █
const G_LOW: u8 = 0xDC; // ▄
const G_SQ: u8 = 0xFE; // ■
const G_LIGHT: u8 = 0xB0; // ░
const G_MED: u8 = 0xB1; // ▒
const G_HEAVY: u8 = 0xB2; // ▓
const G_UP: u8 = 0x1E; // ▲
const G_DOWN: u8 = 0x1F; // ▼

// ---------------------------------------------------------------------------
// Small fixed-size string so we never need alloc
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
pub struct Label<const N: usize> {
    buf: [u8; N],
    len: usize,
}

impl<const N: usize> Label<N> {
    pub const EMPTY: Self = Self { buf: [0; N], len: 0 };

    pub fn from_str(s: &str) -> Self {
        Self::from_bytes(s.as_bytes())
    }

    /// Copies up to N bytes, stops at the first NUL (CPUID brand strings are NUL padded).
    pub fn from_bytes(b: &[u8]) -> Self {
        let mut l = Self::EMPTY;
        for &c in b.iter().take(N) {
            if c == 0 {
                break;
            }
            l.buf[l.len] = c;
            l.len += 1;
        }
        l
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("?").trim()
    }
}

impl<const N: usize> Display for Label<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// Data the platform hands us
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ProcState {
    Running,
    Ready,
    Sleeping,
    Blocked,
    Zombie,
}

impl ProcState {
    fn tag(self) -> &'static str {
        match self {
            ProcState::Running => "RUN",
            ProcState::Ready => "RDY",
            ProcState::Sleeping => "SLP",
            ProcState::Blocked => "BLK",
            ProcState::Zombie => "ZMB",
        }
    }
}

#[derive(Clone, Copy)]
pub struct ProcInfo {
    pub pid: u32,
    pub name: Label<16>,
    pub state: ProcState,
    /// Cumulative timer ticks this task has been the running task.
    pub cpu_ticks: u64,
    pub mem_bytes: u64,
}

impl ProcInfo {
    pub const EMPTY: Self = Self {
        pid: 0,
        name: Label::EMPTY,
        state: ProcState::Zombie,
        cpu_ticks: 0,
        mem_bytes: 0,
    };
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum NicKind {
    Ethernet,
    WiFi,
}

#[derive(Clone, Copy)]
pub struct NicInfo {
    pub name: Label<8>,
    pub kind: NicKind,
    pub link_up: bool,
    pub tx_bytes: u64,
    pub rx_bytes: u64,
    pub tx_packets: u64,
    pub rx_packets: u64,
}

impl NicInfo {
    pub const EMPTY: Self = Self {
        name: Label::EMPTY,
        kind: NicKind::Ethernet,
        link_up: false,
        tx_bytes: 0,
        rx_bytes: 0,
        tx_packets: 0,
        rx_packets: 0,
    };
}

#[derive(Clone, Copy)]
pub struct GpuInfo {
    pub name: Label<32>,
    pub fb_width: u32,
    pub fb_height: u32,
    pub fb_bpp: u32,
    pub vram_bytes: u64,
    /// None = no driver can report load (the normal case on bare metal / QEMU).
    pub usage_pct: Option<u8>,
}

#[derive(Clone, Copy)]
pub struct MemInfo {
    pub total: u64,
    pub used: u64,
    pub heap_total: u64,
    pub heap_used: u64,
}

pub trait Platform {
    /// Monotonic timer ticks since boot.
    fn ticks(&self) -> u64;
    fn ticks_per_sec(&self) -> u64;

    fn cpu_brand(&self) -> Label<48>;
    fn cpu_count(&self) -> usize;
    /// Cumulative ticks core `core` spent in the idle task.
    fn cpu_idle_ticks(&self, core: usize) -> u64;

    fn mem(&self) -> MemInfo;
    fn gpu(&self) -> Option<GpuInfo>;

    /// Fill `out`, return how many entries were written.
    fn nics(&self, out: &mut [NicInfo]) -> usize;
    fn procs(&self, out: &mut [ProcInfo]) -> usize;

    /// Sleep until the next interrupt (interrupts must be enabled).
    fn idle(&self) {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            core::arch::asm!("hlt", options(nomem, nostack));
        }
        #[cfg(not(target_arch = "x86_64"))]
        core::hint::spin_loop();
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Char(u8),
    Up,
    Down,
    Esc,
}

pub trait Term: Write {
    /// (columns, rows)
    fn size(&self) -> (usize, usize);
    fn clear(&mut self);
    fn goto(&mut self, row: usize, col: usize);
    fn color(&mut self, fg: u8, bg: u8);
    /// Write one raw CP437 byte at the cursor and advance (always advances, even if clipped).
    fn put_raw(&mut self, b: u8);
    /// Current cursor column.
    fn col(&self) -> usize;
    /// Ignore drawing at columns >= end_col until changed (cursor still advances).
    fn clip(&mut self, end_col: usize);
    fn read_key(&mut self) -> Option<Key>;
}

macro_rules! put {
    ($t:expr, $($a:tt)*) => {{ let _ = write!($t, $($a)*); }};
}

// ---------------------------------------------------------------------------
// Formatting helpers
// ---------------------------------------------------------------------------

struct Buf {
    b: [u8; 32],
    n: usize,
}

impl Buf {
    fn new() -> Self {
        Self { b: [0; 32], n: 0 }
    }
    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.b[..self.n]).unwrap_or("")
    }
}

impl Write for Buf {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for &c in s.as_bytes() {
            if self.n < self.b.len() {
                self.b[self.n] = c;
                self.n += 1;
            }
        }
        Ok(())
    }
}

fn human(x: u64, w: &mut Buf) {
    const U: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    if x < 1024 {
        let _ = write!(w, "{} B", x);
        return;
    }
    let mut v = x.saturating_mul(10);
    let mut unit = 0;
    while v >= 10240 && unit < 3 {
        v /= 1024;
        unit += 1;
    }
    let _ = write!(w, "{}.{} {}", v / 10, v % 10, U[unit]);
}

struct Bytes(u64);
impl Display for Bytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut b = Buf::new();
        human(self.0, &mut b);
        f.pad(b.as_str())
    }
}

struct Rate(u64);
impl Display for Rate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut b = Buf::new();
        human(self.0, &mut b);
        let _ = b.write_str("/s");
        f.pad(b.as_str())
    }
}

fn level(pct: u32) -> u8 {
    if pct < 50 {
        GREEN
    } else if pct < 80 {
        YELLOW
    } else {
        RED
    }
}

fn push<T: Copy>(h: &mut [T], v: T) {
    h.copy_within(1.., 0);
    let n = h.len();
    h[n - 1] = v;
}

// ---------------------------------------------------------------------------
// Drawing primitives
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct Rect {
    x: usize,
    y: usize,
    w: usize,
    h: usize,
}

/// Start a content line: position the cursor and clip at `end`.
fn begin<T: Term>(t: &mut T, y: usize, x: usize, end: usize) {
    t.goto(y, x);
    t.clip(end);
    t.color(GRAY, BLACK);
}

/// Pad the rest of a content line with spaces and lift the clip.
fn finish<T: Term>(t: &mut T, end: usize, cols: usize, bg: u8) {
    t.color(GRAY, bg);
    while t.col() < end {
        t.put_raw(b' ');
    }
    t.clip(cols);
}

fn border_line<T: Term>(t: &mut T, r: Rect, y: usize, bc: u8, l: u8, rc: u8, ltitle: &str, rtitle: &str) {
    let rtitle = if rtitle.len() + ltitle.len() + 8 > r.w { "" } else { rtitle };
    t.goto(y, r.x);
    t.color(bc, BLACK);
    t.put_raw(l);
    if !ltitle.is_empty() {
        t.put_raw(G_H);
        t.put_raw(G_RT);
        for (i, ch) in ltitle.bytes().enumerate() {
            t.color(if i == 0 { RED } else { WHITE }, BLACK);
            t.put_raw(ch);
        }
        t.color(bc, BLACK);
        t.put_raw(G_LT);
    }
    let rlen = if rtitle.is_empty() { 0 } else { rtitle.len() + 3 };
    let stop = (r.x + r.w - 1).saturating_sub(rlen);
    while t.col() < stop {
        t.put_raw(G_H);
    }
    if rlen > 0 {
        t.put_raw(G_RT);
        t.color(WHITE, BLACK);
        for ch in rtitle.bytes() {
            t.put_raw(ch);
        }
        t.color(bc, BLACK);
        t.put_raw(G_LT);
        t.put_raw(G_H);
    }
    t.put_raw(rc);
}

fn draw_box<T: Term>(t: &mut T, r: Rect, bc: u8, title: &str, top_right: &str, bot_right: &str) {
    t.clip(r.x + r.w);
    border_line(t, r, r.y, bc, G_TL, G_TR, title, top_right);
    border_line(t, r, r.y + r.h - 1, bc, G_BL, G_BR, "", bot_right);
    t.color(bc, BLACK);
    for y in r.y + 1..r.y + r.h - 1 {
        t.goto(y, r.x);
        t.put_raw(G_V);
        t.goto(y, r.x + r.w - 1);
        t.put_raw(G_V);
    }
}

/// btop-style meter: row of ■, coloured green -> yellow -> red by position.
fn meter<T: Term>(t: &mut T, pct: u32, w: usize, bg: u8) {
    let pct = pct.min(100) as usize;
    let filled = (pct * w + 50) / 100;
    for i in 0..w {
        if i < filled {
            t.color(level((i * 100 / w.max(1)) as u32), bg);
        } else {
            t.color(DARK, bg);
        }
        t.put_raw(G_SQ);
    }
}

/// One row of a 2-levels-per-row area graph (uses █ and ▄).
fn graph_row<T: Term>(t: &mut T, row: usize, h: usize, data: &[u8]) {
    let k = h - 1 - row; // cell index counted from the bottom
    t.color(level((k * 100 / h) as u32), BLACK);
    for &v in data {
        let mut u = v.min(100) as usize * (2 * h) / 100;
        if v > 0 && u == 0 {
            u = 1;
        }
        let g = if u >= 2 * k + 2 {
            G_FULL
        } else if u == 2 * k + 1 {
            G_LOW
        } else {
            b' '
        };
        t.put_raw(g);
    }
}

/// Single-row density graph (' ' ░ ▒ ▓ █), auto-scaled with a 1 KiB/s floor.
fn shade_row<T: Term>(t: &mut T, data: &[u32], fg: u8) {
    let max = data.iter().copied().max().unwrap_or(0).max(1024) as u64;
    t.color(fg, BLACK);
    for &v in data {
        let mut l = (v as u64 * 4 / max) as usize;
        if v > 0 && l == 0 {
            l = 1;
        }
        t.put_raw([b' ', G_LIGHT, G_MED, G_HEAVY, G_FULL][l.min(4)]);
    }
}

// ---------------------------------------------------------------------------
// Sampling
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct Snapshot {
    ticks: u64,
    ncpu: usize,
    idle: [u64; MAX_CPUS],
    nnic: usize,
    nics: [NicInfo; MAX_NICS],
    nproc: usize,
    procs: [ProcInfo; MAX_PROCS],
}

fn take<P: Platform>(p: &P) -> Snapshot {
    let mut s = Snapshot {
        ticks: p.ticks(),
        ncpu: p.cpu_count().clamp(1, MAX_CPUS),
        idle: [0; MAX_CPUS],
        nnic: 0,
        nics: [NicInfo::EMPTY; MAX_NICS],
        nproc: 0,
        procs: [ProcInfo::EMPTY; MAX_PROCS],
    };
    for i in 0..s.ncpu {
        s.idle[i] = p.cpu_idle_ticks(i);
    }
    s.nnic = p.nics(&mut s.nics).min(MAX_NICS);
    s.nproc = p.procs(&mut s.procs).min(MAX_PROCS);
    s
}

/// Per-core busy % and the all-core average, over the interval prev -> cur.
fn cpu_pcts(prev: &Snapshot, cur: &Snapshot) -> ([u32; MAX_CPUS], u32) {
    let dt = cur.ticks.saturating_sub(prev.ticks).max(1);
    let mut core = [0u32; MAX_CPUS];
    let mut total = 0u32;
    for i in 0..cur.ncpu {
        let idle = cur.idle[i].saturating_sub(prev.idle[i]).min(dt);
        core[i] = ((dt - idle) * 100 / dt) as u32;
        total += core[i];
    }
    (core, total / cur.ncpu as u32)
}

/// (rx, tx) bytes/sec for NIC `i` over prev -> cur.
fn nic_rates(prev: &Snapshot, cur: &Snapshot, i: usize, tps: u64) -> (u64, u64) {
    let dt = cur.ticks.saturating_sub(prev.ticks).max(1);
    let n = &cur.nics[i];
    let (mut orx, mut otx) = (n.rx_bytes, n.tx_bytes);
    for j in 0..prev.nnic {
        if prev.nics[j].name.as_str() == n.name.as_str() {
            orx = prev.nics[j].rx_bytes;
            otx = prev.nics[j].tx_bytes;
            break;
        }
    }
    (
        n.rx_bytes.saturating_sub(orx).saturating_mul(tps) / dt,
        n.tx_bytes.saturating_sub(otx).saturating_mul(tps) / dt,
    )
}

// ---------------------------------------------------------------------------
// State + main loop
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Sort {
    Cpu,
    Mem,
    Pid,
}

struct Ui {
    sort: Sort,
    sel: usize,
    top: usize,
    hist: [u8; HIST],
    net_rx: [[u32; HIST]; MAX_NICS],
    net_tx: [[u32; HIST]; MAX_NICS],
    brand: Label<48>,
}

pub fn run<P: Platform, T: Term>(p: &mut P, t: &mut T) {
    let tps = p.ticks_per_sec().max(1);
    let mut ui = Ui {
        sort: Sort::Cpu,
        sel: 0,
        top: 0,
        hist: [0; HIST],
        net_rx: [[0; HIST]; MAX_NICS],
        net_tx: [[0; HIST]; MAX_NICS],
        brand: p.cpu_brand(),
    };

    let mut cur = take(p);
    let mut prev = cur;
    let mut next = cur.ticks + tps;
    let mut dirty = true;

    t.clip(usize::MAX);
    t.color(GRAY, BLACK);
    t.clear();

    loop {
        while let Some(k) = t.read_key() {
            match k {
                Key::Char(b'q') | Key::Char(b'Q') | Key::Esc => {
                    t.clip(usize::MAX);
                    t.color(GRAY, BLACK);
                    t.clear();
                    t.goto(0, 0);
                    return;
                }
                Key::Char(b'c') => ui.sort = Sort::Cpu,
                Key::Char(b'm') => ui.sort = Sort::Mem,
                Key::Char(b'p') => ui.sort = Sort::Pid,
                Key::Up => ui.sel = ui.sel.saturating_sub(1),
                Key::Down => ui.sel += 1, // clamped in draw_proc()
                _ => {}
            }
            dirty = true;
        }

        let now = p.ticks();
        if now >= next {
            prev = core::mem::replace(&mut cur, take(p));
            let (_, total) = cpu_pcts(&prev, &cur);
            push(&mut ui.hist, total as u8);
            for i in 0..cur.nnic {
                let (rx, tx) = nic_rates(&prev, &cur, i, tps);
                push(&mut ui.net_rx[i], rx.min(u32::MAX as u64) as u32);
                push(&mut ui.net_tx[i], tx.min(u32::MAX as u64) as u32);
            }
            next = now + tps;
            dirty = true;
        }

        if dirty {
            draw(p, t, &mut ui, &prev, &cur);
            dirty = false;
        }

        p.idle();
    }
}

// ---------------------------------------------------------------------------
// Panels
// ---------------------------------------------------------------------------

fn draw<P: Platform, T: Term>(p: &P, t: &mut T, ui: &mut Ui, prev: &Snapshot, cur: &Snapshot) {
    let (cols, rows) = t.size();
    t.clip(cols);
    if cols < 60 || rows < 20 {
        t.goto(0, 0);
        t.color(RED, BLACK);
        put!(t, "tskmngr: terminal too small (need 60x20)");
        return;
    }
    let tps = p.ticks_per_sec().max(1);

    let cpu_h = (rows / 3).clamp(6, 10);
    let lw = (cols * 2 / 5).max(28).min(cols - 28);
    let body_h = rows - cpu_h;
    let net_h = body_h.saturating_sub(9).max(3);

    let (core, total) = cpu_pcts(prev, cur);
    draw_cpu(
        t,
        Rect { x: 0, y: 0, w: cols, h: cpu_h },
        cols,
        ui,
        cur,
        &core,
        total,
        cur.ticks / tps,
    );
    draw_mem(t, Rect { x: 0, y: cpu_h, w: lw, h: 5 }, cols, &p.mem());
    draw_gpu(t, Rect { x: 0, y: cpu_h + 5, w: lw, h: 4 }, cols, p.gpu());
    draw_net(t, Rect { x: 0, y: cpu_h + 9, w: lw, h: net_h }, cols, ui, cur);

    // ---- per-process CPU% and ordering ----
    let dt = cur.ticks.saturating_sub(prev.ticks).max(1);
    let np = cur.nproc.min(MAX_PROCS);
    let ncpu = cur.ncpu as u64;
    let mut pct = [0u32; MAX_PROCS];
    for i in 0..np {
        let pr = &cur.procs[i];
        let mut old = pr.cpu_ticks;
        for j in 0..prev.nproc.min(MAX_PROCS) {
            if prev.procs[j].pid == pr.pid {
                old = prev.procs[j].cpu_ticks;
                break;
            }
        }
        pct[i] = (pr.cpu_ticks.saturating_sub(old).saturating_mul(100) / (dt * ncpu)).min(100) as u32;
    }

    let sort = ui.sort;
    let better = |a: usize, b: usize| -> bool {
        match sort {
            Sort::Cpu => (pct[a], cur.procs[a].mem_bytes) > (pct[b], cur.procs[b].mem_bytes),
            Sort::Mem => cur.procs[a].mem_bytes > cur.procs[b].mem_bytes,
            Sort::Pid => cur.procs[a].pid < cur.procs[b].pid,
        }
    };
    let mut order = [0usize; MAX_PROCS];
    for i in 0..np {
        order[i] = i;
    }
    for i in 1..np {
        let mut j = i;
        while j > 0 && better(order[j], order[j - 1]) {
            order.swap(j, j - 1);
            j -= 1;
        }
    }

    draw_proc(
        t,
        Rect { x: lw, y: cpu_h, w: cols - lw, h: body_h },
        cols,
        ui,
        cur,
        &pct,
        &order,
        np,
    );
}

fn draw_cpu<T: Term>(
    t: &mut T,
    r: Rect,
    cols: usize,
    ui: &Ui,
    cur: &Snapshot,
    core: &[u32; MAX_CPUS],
    total: u32,
    up: u64,
) {
    let mut ub = Buf::new();
    put!(ub, "up {:02}:{:02}:{:02}", up / 3600, (up / 60) % 60, up % 60);
    draw_box(t, r, DGREEN, "cpu", "tskmngr", ub.as_str());

    let (ix, iy, iw, ih) = (r.x + 1, r.y + 1, r.w - 2, r.h - 2);
    let gw = (iw * 55 / 100).min(HIST).max(8);
    let rw = iw - gw - 1;
    let cells = (rw / 17).max(1);

    for row in 0..ih {
        begin(t, iy + row, ix, ix + iw);
        graph_row(t, row, ih, &ui.hist[HIST - gw..]);
        t.color(GRAY, BLACK);
        t.put_raw(b' ');
        match row {
            0 => {
                t.color(WHITE, BLACK);
                put!(t, "{}", ui.brand);
            }
            1 => {
                t.color(GRAY, BLACK);
                put!(t, "CPU ");
                meter(t, total, rw.saturating_sub(9), BLACK);
                t.color(level(total), BLACK);
                put!(t, " {:>3}%", total);
            }
            _ => {
                let k = row - 2;
                for c in 0..cells {
                    let idx = k * cells + c;
                    if idx >= cur.ncpu {
                        break;
                    }
                    t.color(GRAY, BLACK);
                    put!(t, "c{:<2} ", idx);
                    meter(t, core[idx], 6, BLACK);
                    t.color(level(core[idx]), BLACK);
                    put!(t, " {:>3}%  ", core[idx]);
                }
            }
        }
        finish(t, ix + iw, cols, BLACK);
    }
}

fn draw_mem<T: Term>(t: &mut T, r: Rect, cols: usize, m: &MemInfo) {
    draw_box(t, r, BROWN, "mem", "", "");
    let (ix, iy, iw) = (r.x + 1, r.y + 1, r.w - 2);
    let bw = iw.saturating_sub(10);
    let rp = (m.used * 100 / m.total.max(1)).min(100) as u32;
    let hp = (m.heap_used * 100 / m.heap_total.max(1)).min(100) as u32;

    begin(t, iy, ix, ix + iw);
    put!(t, "RAM  ");
    meter(t, rp, bw, BLACK);
    t.color(level(rp), BLACK);
    put!(t, " {:>3}%", rp);
    finish(t, ix + iw, cols, BLACK);

    begin(t, iy + 1, ix, ix + iw);
    t.color(DARK, BLACK);
    put!(t, "     {} / {}", Bytes(m.used), Bytes(m.total));
    finish(t, ix + iw, cols, BLACK);

    begin(t, iy + 2, ix, ix + iw);
    put!(t, "Heap ");
    meter(t, hp, bw, BLACK);
    t.color(level(hp), BLACK);
    put!(t, " {:>3}%", hp);
    finish(t, ix + iw, cols, BLACK);
}

fn draw_gpu<T: Term>(t: &mut T, r: Rect, cols: usize, g: Option<GpuInfo>) {
    draw_box(t, r, MAGENTA, "gpu", "", "");
    let (ix, iy, iw) = (r.x + 1, r.y + 1, r.w - 2);

    begin(t, iy, ix, ix + iw);
    match g {
        Some(g) => {
            t.color(WHITE, BLACK);
            put!(t, "{}", g.name);
            finish(t, ix + iw, cols, BLACK);

            begin(t, iy + 1, ix, ix + iw);
            put!(t, "{}x{} {}bpp  ", g.fb_width, g.fb_height, g.fb_bpp);
            match g.usage_pct {
                Some(u) => {
                    meter(t, u as u32, 8, BLACK);
                    t.color(level(u as u32), BLACK);
                    put!(t, " {:>3}%", u);
                }
                None => {
                    t.color(DARK, BLACK);
                    put!(t, "load n/a");
                }
            }
            finish(t, ix + iw, cols, BLACK);
        }
        None => {
            t.color(DARK, BLACK);
            put!(t, "no display adapter");
            finish(t, ix + iw, cols, BLACK);
            begin(t, iy + 1, ix, ix + iw);
            finish(t, ix + iw, cols, BLACK);
        }
    }
}

fn draw_net<T: Term>(t: &mut T, r: Rect, cols: usize, ui: &Ui, cur: &Snapshot) {
    draw_box(t, r, DRED, "net", "", "");
    let (ix, iy, iw, ih) = (r.x + 1, r.y + 1, r.w - 2, r.h - 2);
    let end = ix + iw;
    let gw = iw.saturating_sub(14).min(HIST);

    let (mut has_eth, mut has_wifi) = (false, false);
    for i in 0..cur.nnic {
        match cur.nics[i].kind {
            NicKind::Ethernet => has_eth = true,
            NicKind::WiFi => has_wifi = true,
        }
    }

    let mut line = 0usize;
    for i in 0..cur.nnic {
        if line + 3 > ih {
            break;
        }
        let n = &cur.nics[i];

        begin(t, iy + line, ix, end);
        t.color(WHITE, BLACK);
        put!(t, "{:<5}", n.name);
        if n.link_up {
            t.color(GREEN, BLACK);
            put!(t, "UP   ");
        } else {
            t.color(RED, BLACK);
            put!(t, "DOWN ");
        }
        t.color(DARK, BLACK);
        put!(t, "{}/{}", Bytes(n.rx_bytes), Bytes(n.tx_bytes));
        finish(t, end, cols, BLACK);
        line += 1;

        begin(t, iy + line, ix, end);
        t.color(GREEN, BLACK);
        t.put_raw(G_DOWN);
        t.color(WHITE, BLACK);
        put!(t, " {:>11} ", Rate(ui.net_rx[i][HIST - 1] as u64));
        shade_row(t, &ui.net_rx[i][HIST - gw..], GREEN);
        finish(t, end, cols, BLACK);
        line += 1;

        begin(t, iy + line, ix, end);
        t.color(RED, BLACK);
        t.put_raw(G_UP);
        t.color(WHITE, BLACK);
        put!(t, " {:>11} ", Rate(ui.net_tx[i][HIST - 1] as u64));
        shade_row(t, &ui.net_tx[i][HIST - gw..], RED);
        finish(t, end, cols, BLACK);
        line += 1;
    }

    if !has_eth && line < ih {
        begin(t, iy + line, ix, end);
        t.color(DARK, BLACK);
        put!(t, "eth0  no device");
        finish(t, end, cols, BLACK);
        line += 1;
    }
    if !has_wifi && line < ih {
        begin(t, iy + line, ix, end);
        t.color(DARK, BLACK);
        put!(t, "wifi0 no device");
        finish(t, end, cols, BLACK);
        line += 1;
    }
    while line < ih {
        begin(t, iy + line, ix, end);
        finish(t, end, cols, BLACK);
        line += 1;
    }
}

fn scroll_glyph(i: usize, list_h: usize, np: usize, top: usize) -> (u8, u8) {
    if np <= list_h || list_h == 0 {
        return (b' ', GRAY);
    }
    let tl = (list_h * list_h / np).max(1);
    let tp = top * (list_h - tl) / (np - list_h);
    if i >= tp && i < tp + tl {
        (G_FULL, GRAY)
    } else {
        (G_LIGHT, DARK)
    }
}

fn draw_proc<T: Term>(
    t: &mut T,
    r: Rect,
    cols: usize,
    ui: &mut Ui,
    cur: &Snapshot,
    pct: &[u32; MAX_PROCS],
    order: &[usize; MAX_PROCS],
    np: usize,
) {
    let mut tb = Buf::new();
    put!(tb, "{} tasks", np);
    draw_box(t, r, CYAN, "proc", tb.as_str(), "q quit  c/m/p sort  up/dn");

    let (ix, iy, iw, ih) = (r.x + 1, r.y + 1, r.w - 2, r.h - 2);
    let usable = iw - 1; // last inner column is the scrollbar
    let end = ix + usable;
    let nw = usable.saturating_sub(31).max(4);
    let list_h = ih.saturating_sub(1);

    // ---- header ----
    begin(t, iy, ix, end);
    let s = ui.sort;
    t.color(WHITE, BLACK);
    put!(t, " ");
    t.color(if s == Sort::Pid { YELLOW } else { WHITE }, BLACK);
    put!(t, "{:>5} ", "PID");
    t.color(WHITE, BLACK);
    put!(t, "{:<w$} ", "NAME", w = nw);
    put!(t, "{:<3} ", "ST");
    t.color(if s == Sort::Mem { YELLOW } else { WHITE }, BLACK);
    put!(t, "{:>9} ", "MEM");
    put!(t, "     ");
    t.color(if s == Sort::Cpu { YELLOW } else { WHITE }, BLACK);
    put!(t, "{:>4}", "CPU%");
    finish(t, end, cols, BLACK);
    t.goto(iy, end);
    t.put_raw(b' ');

    // ---- selection / scroll ----
    ui.sel = ui.sel.min(np.saturating_sub(1));
    if ui.sel < ui.top {
        ui.top = ui.sel;
    }
    if list_h > 0 && ui.sel >= ui.top + list_h {
        ui.top = ui.sel + 1 - list_h;
    }
    ui.top = ui.top.min(np.saturating_sub(list_h));

    // ---- rows ----
    for i in 0..list_h {
        let y = iy + 1 + i;
        let idx = ui.top + i;
        if idx < np {
            let k = order[idx];
            let pr = &cur.procs[k];
            let sel = idx == ui.sel;
            let bg = if sel { BLUE } else { BLACK };

            begin(t, y, ix, end);
            t.color(GRAY, bg);
            put!(t, " ");
            t.color(if sel { WHITE } else { GRAY }, bg);
            put!(t, "{:>5} ", pr.pid);
            t.color(if sel || pr.state == ProcState::Running { WHITE } else { GRAY }, bg);
            put!(t, "{:<w$.w$} ", pr.name, w = nw);
            t.color(
                match pr.state {
                    ProcState::Running => GREEN,
                    ProcState::Zombie => RED,
                    ProcState::Blocked => YELLOW,
                    _ => {
                        if sel {
                            WHITE
                        } else {
                            GRAY
                        }
                    }
                },
                bg,
            );
            put!(t, "{:<3} ", pr.state.tag());
            t.color(if sel { WHITE } else { GRAY }, bg);
            put!(t, "{:>9} ", Bytes(pr.mem_bytes));
            meter(t, pct[k], 4, bg);
            t.color(level(pct[k]), bg);
            put!(t, " {:>3}%", pct[k]);
            finish(t, end, cols, bg);
        } else {
            begin(t, y, ix, end);
            finish(t, end, cols, BLACK);
        }
        let (g, c) = scroll_glyph(i, list_h, np, ui.top);
        t.goto(y, end);
        t.color(c, BLACK);
        t.put_raw(g);
    }
}
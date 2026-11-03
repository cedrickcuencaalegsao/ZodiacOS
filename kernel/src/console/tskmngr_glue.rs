//! kernel/src/console/tskmngr_glue.rs
//!
//! Connects tskmngr to ZodiacOS. Every line marked `WIRE:` calls something I
//! can't see in your tree - rename it to whatever your kernel actually exposes.
//!
//! mod.rs:
//!     pub mod tskmngr;
//!     pub mod tskmngr_glue;
//!
//! shell dispatch:
//!     "tskmngr" | "top" | "btop" => {
//!         crate::console::tskmngr_glue::run_tskmngr();
//!         // WIRE: clear / redraw your fbcon here (tskmngr painted over it)
//!     }

use super::tskmngr::*;
use crate::drivers::framebuffer::{self, Framebuffer};
use crate::process::task::{name_str, State};
use core::fmt;

// ===========================================================================
// 0. Entry point: picks the framebuffer terminal, falls back to VGA text mode
// ===========================================================================



// ===========================================================================
// 1. Counters the kernel has to maintain (add these, then WIRE below)
// ===========================================================================
//
// --- timer IRQ handler (per tick) ---
//   use core::sync::atomic::{AtomicU64, Ordering};
//   pub static TICKS: AtomicU64 = AtomicU64::new(0);
//   pub static IDLE_TICKS: [AtomicU64; 16] = [const { AtomicU64::new(0) }; 16];
//
//   TICKS.fetch_add(1, Ordering::Relaxed);
//   let cur = scheduler::current_task();
//   if cur.is_idle { IDLE_TICKS[cpu_id].fetch_add(1, Ordering::Relaxed); }
//   cur.cpu_ticks += 1;                    // per-task, used for the process CPU%
//
// --- e1000 driver (and later your wifi driver) ---
//   pub struct NetStats {
//       pub tx_bytes: AtomicU64, pub rx_bytes: AtomicU64,
//       pub tx_packets: AtomicU64, pub rx_packets: AtomicU64,
//   }
//   pub static STATS: NetStats = NetStats { /* all AtomicU64::new(0) */ };
//
//   in send():          STATS.tx_bytes.fetch_add(len as u64, Relaxed); STATS.tx_packets.fetch_add(1, Relaxed);
//   in the RX handler:  STATS.rx_bytes.fetch_add(len as u64, Relaxed); STATS.rx_packets.fetch_add(1, Relaxed);

// ===========================================================================
// 2. Platform
// ===========================================================================

pub struct ZodiacPlatform;

impl Platform for ZodiacPlatform {
    fn ticks(&self) -> u64 {
        crate::interrupts::timer::ticks() // WIRE
    }

    fn ticks_per_sec(&self) -> u64 {
        crate::interrupts::timer::HZ as u64 // WIRE (PIT/APIC timer frequency you programmed)
    }

    fn cpu_brand(&self) -> Label<48> {
        cpu_brand_string()
    }

    fn cpu_count(&self) -> usize {
        1 // WIRE: number of online cores once you bring up SMP
    }

    fn cpu_idle_ticks(&self, _core: usize) -> u64 {
        let mut idle = 0;
        crate::scheduler::for_each_task(|t| {
            // pid 0 is my guess for the idle task; the name check is the fallback
            if t.pid == 0 || name_str(&t.name) == "idle" {
                idle = t.cpu_ticks;
            }
        });
        idle
    }

    fn mem(&self) -> MemInfo {
        MemInfo {
            total: crate::pmm::total_bytes(),
            used: crate::pmm::used_bytes(),
            heap_total: crate::memory::heap::size() as u64,
            heap_used: crate::memory::heap::used() as u64,
        }
    }

    fn gpu(&self) -> Option<GpuInfo> {
        let fb = framebuffer::info()?;
        let name = match crate::drivers::pci::find_class(0x03, 0x00) // 0x03/0x00 = VGA-compatible controller
            .or_else(|| crate::drivers::pci::find_class(0x03, 0x80)) // 0x03/0x80 = other display controller
        {
            Some(d) => gpu_name(d.vendor, d.device),
            None => "Generic framebuffer",
        };
        Some(GpuInfo {
            name: Label::from_str(name),
            fb_width: fb.width as u32,
            fb_height: fb.height as u32,
            fb_bpp: fb.bpp as u32,
            // every page the driver can flip between, not just the visible one
            vram_bytes: (fb.pitch * fb.height * fb.pages.max(1)) as u64,
            usage_pct: None, // needs a real GPU driver
        })
    }

    fn nics(&self, out: &mut [NicInfo]) -> usize {
        let mut n = 0;
        crate::drivers::network::for_each_device(|dev| {
            if n >= out.len() {
                return;
            }
            let s = dev.stats();
            let name = dev.name();
            out[n] = NicInfo {
                name: Label::from_str(name),
                kind: if name.starts_with("wlan") || name.starts_with("wifi") {
                    NicKind::WiFi
                } else {
                    NicKind::Ethernet
                },
                link_up: dev.link_up(),
                tx_bytes: s.tx_bytes,
                rx_bytes: s.rx_bytes,
                tx_packets: s.tx_packets,
                rx_packets: s.rx_packets,
            };
            n += 1;
        });
        n
    }

    fn procs(&self, out: &mut [ProcInfo]) -> usize {
        let mut n = 0;
        crate::scheduler::for_each_task(|t| {
            if t.state == State::Free || n >= out.len() {
                return;
            }
            out[n] = ProcInfo {
                pid: t.pid,
                name: Label::from_str(name_str(&t.name)),
                state: match t.state {
                    State::Running => ProcState::Running,
                    State::Ready | State::Embryo => ProcState::Ready,
                    State::Sleeping => ProcState::Sleeping,
                    State::Blocked => ProcState::Blocked,
                    State::Zombie | State::Free => ProcState::Zombie,
                },
                cpu_ticks: t.cpu_ticks,
                // Kernel stack size only for now; swap in mapped user pages later.
                mem_bytes: t.kstack_top.saturating_sub(t.kstack_base),
            };
            n += 1;
        });
        n
    }

    /// `sti; hlt` so the CPU can never sleep with interrupts disabled
    /// (the instruction after `sti` runs before any interrupt, so no wakeup is lost).
    fn idle(&self) {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            core::arch::asm!("sti", "hlt", options(nomem, nostack));
        }
        #[cfg(not(target_arch = "x86_64"))]
        core::hint::spin_loop();
    }
}

#[cfg(target_arch = "x86_64")]
fn cpu_brand_string() -> Label<48> {
    use core::arch::x86_64::__cpuid;
    let mut b = [0u8; 48];
    #[allow(unused_unsafe)]
    unsafe {
        if __cpuid(0x8000_0000).eax >= 0x8000_0004 {
            for i in 0..3u32 {
                let r = __cpuid(0x8000_0002 + i);
                let o = i as usize * 16;
                b[o..o + 4].copy_from_slice(&r.eax.to_le_bytes());
                b[o + 4..o + 8].copy_from_slice(&r.ebx.to_le_bytes());
                b[o + 8..o + 12].copy_from_slice(&r.ecx.to_le_bytes());
                b[o + 12..o + 16].copy_from_slice(&r.edx.to_le_bytes());
            }
        }
    }
    Label::from_bytes(&b)
}

#[cfg(not(target_arch = "x86_64"))]
fn cpu_brand_string() -> Label<48> {
    Label::from_str("unknown CPU")
}

fn gpu_name(vendor: u16, device: u16) -> &'static str {
    match (vendor, device) {
        (0x1234, 0x1111) => "QEMU Standard VGA",
        (0x15ad, 0x0405) => "VMware SVGA II",
        (0x80ee, 0xbeef) => "VirtualBox Graphics",
        (0x1af4, 0x1050) => "virtio-gpu",
        (0x8086, _) => "Intel Graphics",
        (0x10de, _) => "NVIDIA GPU",
        (0x1002, _) => "AMD/ATI GPU",
        _ => "Display controller",
    }
}

// ===========================================================================
// 3. Framebuffer terminal (32-bit linear framebuffer + the captured 8x16 font)
// ===========================================================================
// This is what you want when ZodiacOS is in graphics mode. The font comes from
// the BIOS, so it is CP437 and the box/block glyphs draw correctly.
// Screen size is (width / 8) x (height / 16) cells, e.g. 128x48 at 1024x768.

const PALETTE: [u32; 16] = [
    0x000000, 0x0000AA, 0x00AA00, 0x00AAAA, 0xAA0000, 0xAA00AA, 0xAA5500, 0xAAAAAA,
    0x555555, 0x5555FF, 0x55FF55, 0x55FFFF, 0xFF5555, 0xFF55FF, 0xFFFF55, 0xFFFFFF,
];

pub struct FbTerm {
    fb: Framebuffer,
    cols: usize,
    rows: usize,
    row: usize,
    col: usize,
    fg: u32,
    bg: u32,
    clip_col: usize,
    read: fn() -> Option<Key>,
}

impl FbTerm {
    /// Returns None if the system is still in VGA text mode.
    pub fn new(read: fn() -> Option<Key>) -> Option<Self> {
        let fb = framebuffer::get()?;
        let cols = fb.width / 8;
        let rows = fb.height / 16;
        Some(Self {
            fb,
            cols,
            rows,
            row: 0,
            col: 0,
            fg: PALETTE[7],
            bg: PALETTE[0],
            clip_col: cols,
            read,
        })
    }

    fn put(&self, r: usize, c: usize, ch: u8) {
        if r < self.rows && c < self.cols && c < self.clip_col {
            self.fb
                .draw_glyph(c * 8, r * 16, framebuffer::glyph(ch), self.fg, self.bg);
        }
    }
}

impl fmt::Write for FbTerm {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for b in s.bytes() {
            let ch = if (0x20..0x7f).contains(&b) { b } else { b'?' };
            self.put_raw(ch);
        }
        Ok(())
    }
}

impl Term for FbTerm {
    fn size(&self) -> (usize, usize) {
        (self.cols, self.rows)
    }

    fn clear(&mut self) {
        self.clip_col = self.cols;
        self.fb
            .fill_rect(0, 0, self.fb.width, self.fb.height, PALETTE[0]);
        self.row = 0;
        self.col = 0;
    }

    fn goto(&mut self, row: usize, col: usize) {
        self.row = row;
        self.col = col;
    }

    fn color(&mut self, fg: u8, bg: u8) {
        self.fg = PALETTE[(fg & 0x0f) as usize];
        self.bg = PALETTE[(bg & 0x0f) as usize];
    }

    fn put_raw(&mut self, b: u8) {
        self.put(self.row, self.col, b);
        if self.col < self.cols {
            self.col += 1;
        }
    }

    fn col(&self) -> usize {
        self.col
    }

    fn clip(&mut self, end_col: usize) {
        self.clip_col = end_col.min(self.cols);
    }

    fn read_key(&mut self) -> Option<Key> {
        (self.read)()
    }
}

// ===========================================================================
// 4. VGA text-mode terminal (80x25 @ 0xB8000) - fallback only
// ===========================================================================
// Only visible if the system is really in VGA text mode (no framebuffer).

const COLS: usize = 80;
const ROWS: usize = 25;
const VGA_BASE: *mut u16 = 0xb8000 as *mut u16;

pub struct VgaTerm {
    row: usize,
    col: usize,
    attr: u8,
    clip_col: usize,
    read: fn() -> Option<Key>,
}

impl VgaTerm {
    /// `read` must be a non-blocking keyboard poll returning Esc/Up/Down/Char.
    pub fn new(read: fn() -> Option<Key>) -> Self {
        Self {
            row: 0,
            col: 0,
            attr: 0x07,
            clip_col: COLS,
            read,
        }
    }

    fn put(&self, r: usize, c: usize, ch: u8) {
        if r < ROWS && c < COLS && c < self.clip_col {
            unsafe {
                VGA_BASE
                    .add(r * COLS + c)
                    .write_volatile(((self.attr as u16) << 8) | ch as u16)
            }
        }
    }
}

impl fmt::Write for VgaTerm {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for b in s.bytes() {
            let ch = if (0x20..0x7f).contains(&b) { b } else { b'?' };
            self.put_raw(ch);
        }
        Ok(())
    }
}

impl Term for VgaTerm {
    fn size(&self) -> (usize, usize) {
        (COLS, ROWS)
    }

    fn clear(&mut self) {
        self.clip_col = COLS;
        for r in 0..ROWS {
            for c in 0..COLS {
                self.put(r, c, b' ');
            }
        }
        self.row = 0;
        self.col = 0;
    }

    fn goto(&mut self, row: usize, col: usize) {
        self.row = row;
        self.col = col;
    }

    fn color(&mut self, fg: u8, bg: u8) {
        self.attr = ((bg & 0x0f) << 4) | (fg & 0x0f);
    }

    /// VGA text mode is CP437 natively, so box/block glyphs go straight in.
    fn put_raw(&mut self, b: u8) {
        self.put(self.row, self.col, b);
        if self.col < COLS {
            self.col += 1;
        }
    }

    fn col(&self) -> usize {
        self.col
    }

    fn clip(&mut self, end_col: usize) {
        self.clip_col = end_col.min(COLS);
    }

    fn read_key(&mut self) -> Option<Key> {
        (self.read)()
    }
}

// ===========================================================================
// 5. Keyboard adapter
// ===========================================================================

// pub fn read_key() -> Option<Key> {
//     let c = keyboard::read_char()?; // WIRE: non-blocking poll
//     Some(match c {
//         27 => Key::Esc,
//         KEY_UP => Key::Up,
//         KEY_DOWN => Key::Down,
//         other => Key::Char(other), // adjust if Key::Char holds a `char`: other as char
//     })
// }
use crate::{log_info, log_warn};

pub mod bochs_vga;
pub mod framebuffer;
pub mod keyboard; // PS/2
pub mod pci;
pub mod rtc; // CMOS clock
pub mod serial; // 16550 on x86, PL011 on AArch64 later
pub mod storage;

/// Used when the card reports no preferred size.
const FALLBACK_WIDTH: u16 = 1920;
const FALLBACK_HEIGHT: u16 = 1080;

/// Limits: fbcon's text grid holds at most 240 columns x 100 rows (1920x1600 px),
/// and QEMU's default video memory is 16 MB.
const MAX_WIDTH: u16 = 1920;
const MAX_HEIGHT: u16 = 1200;

/// Keeps the size inside what the console supports, aligned to the 8x16 font cell.
fn clamp_mode((w, h): (u16, u16)) -> (u16, u16) {
    (w.min(MAX_WIDTH) & !7, h.min(MAX_HEIGHT) & !15)
}

pub fn init() {
    let devices = pci::init();
    log_info!("pci: {} devices", devices);

    // Ask the display card for its preferred size (set from the host screen by the Makefile).
    let (width, height) = match bochs_vga::preferred_mode() {
        Some(mode) => {
            let (w, h) = clamp_mode(mode);
            log_info!("display: preferred mode {}x{} -> using {}x{}", mode.0, mode.1, w, h);
            (w, h)
        }
        None => {
            log_warn!("display: no EDID, using {}x{}", FALLBACK_WIDTH, FALLBACK_HEIGHT);
            (FALLBACK_WIDTH, FALLBACK_HEIGHT)
        }
    };

    match framebuffer::init(width, height) {
        Ok(()) => log_info!("display: {}x{} framebuffer", width, height),
        Err(e) => log_warn!("display: {}, staying in VGA text mode", e),
    }
}
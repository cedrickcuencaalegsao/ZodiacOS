use crate::{log_info, log_warn};

pub mod bochs_vga;
pub mod framebuffer;
pub mod keyboard; // PS/2
pub mod pci;
pub mod rtc; // CMOS clock
pub mod serial; // 16550 on x86, PL011 on AArch64 later
pub mod storage;

/// Screen resolution to request. QEMU's window resizes to match.
const SCREEN_WIDTH: u16 = 1280;
const SCREEN_HEIGHT: u16 = 800;

pub fn init() {
    let devices = pci::init();
    log_info!("pci: {} devices", devices);

    match framebuffer::init(SCREEN_WIDTH, SCREEN_HEIGHT) {
        Ok(()) => log_info!("display: {}x{} framebuffer", SCREEN_WIDTH, SCREEN_HEIGHT),
        Err(e) => log_warn!("display: {}, staying in VGA text mode", e),
    }
}
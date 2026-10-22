use crate::{log_info, log_warn};

pub mod bochs_vga;
pub mod framebuffer;
pub mod keyboard; // PS/2
pub mod mouse;
pub mod network; // e1000 NIC
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
    storage::init();
    let devices = pci::init();
    log_info!("pci: {} devices", devices);
    probe_pci_drivers();

    // Ask the display card for its preferred size (set from the host screen by the Makefile).
    let (width, height) = match bochs_vga::preferred_mode() {
        Some(mode) => {
            let (w, h) = clamp_mode(mode);
            log_info!(
                "display: preferred mode {}x{} -> using {}x{}",
                mode.0,
                mode.1,
                w,
                h
            );
            (w, h)
        }
        None => {
            log_warn!(
                "display: no EDID, using {}x{}",
                FALLBACK_WIDTH,
                FALLBACK_HEIGHT
            );
            (FALLBACK_WIDTH, FALLBACK_HEIGHT)
        }
    };

    match framebuffer::init(width, height) {
        Ok(()) => log_info!("display: {}x{} framebuffer", width, height),
        Err(e) => log_warn!("display: {}, staying in VGA text mode", e),
    }
}

use pci::PciDevice;

pub struct PciDriver {
    pub name: &'static str,
    pub matches: fn(&PciDevice) -> bool,
    pub probe: fn(&PciDevice) -> errors::Result<()>,
}

static PCI_DRIVERS: &[PciDriver] = &[PciDriver {
    name: "e1000",
    matches: network::e1000::matches,
    probe: network::e1000::probe,
}];

fn probe_pci_drivers() {
    for dev in pci::devices() {
        for drv in PCI_DRIVERS {
            if (drv.matches)(&dev) {
                log_info!(
                    "pci: {:02x}:{:02x}.{} -> {}",
                    dev.bus,
                    dev.dev,
                    dev.func,
                    drv.name
                );
                if let Err(e) = (drv.probe)(&dev) {
                    log_warn!("{}: probe failed: {:?}", drv.name, e);
                }
                break;
            }
        }
    }
}

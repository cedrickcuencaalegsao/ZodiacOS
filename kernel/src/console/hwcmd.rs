use crate::console::fbcon;
use crate::drivers::pci;
use crate::kprintln;

#[allow(dead_code)]
pub fn run(cmd: &str) {
    match cmd {
        "lspci" => lspci(),
        "display" => display(),
        _ => {}
    }
}

#[allow(dead_code)]
fn lspci() {
    for d in pci::devices() {
        let name = pci::device_name(d.vendor, d.device)
            .unwrap_or_else(|| pci::class_name(d.class, d.subclass));
        kprintln!(
            "{:02x}:{:02x}.{} {:04x}:{:04x}  {}",
            d.bus, d.dev, d.func, d.vendor, d.device, name
        );
    }
}

#[allow(dead_code)]
fn display() {
    match fbcon::info() {
        Some((w, h, base, cols, rows)) => {
            kprintln!("framebuffer {}x{}, 32 bits per pixel, at {:#x}", w, h, base);
            kprintln!("text console {} columns x {} rows", cols, rows);
        }
        None => kprintln!("VGA text mode, 80 columns x 25 rows"),
    }
}
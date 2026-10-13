use crate::arch::{inl, outl};
use crate::base::sync::SpinLock;
use alloc::vec::Vec;

const CONFIG_ADDRESS: u16 = 0xCF8;
const CONFIG_DATA: u16 = 0xCFC;

#[allow(dead_code)]
#[derive(Clone, Copy)]
pub struct PciDevice {
    pub bus: u8,
    pub dev: u8,
    pub func: u8,
    pub vendor: u16,
    pub device: u16,
    pub class: u8,
    pub subclass: u8,
    pub prog_if: u8,
    pub irq_line: u8,
}

#[allow(dead_code)]
pub enum Bar {
    None,
    Io { port: u16 },
    Memory { base: u64, prefetchable: bool },
}

static DEVICES: SpinLock<Vec<PciDevice>> = SpinLock::new(Vec::new());

fn address(bus: u8, dev: u8, func: u8, offset: u8) -> u32 {
    0x8000_0000
        | (bus as u32) << 16
        | (dev as u32) << 11
        | (func as u32) << 8
        | (offset as u32 & 0xFC)
}

pub fn read32(bus: u8, dev: u8, func: u8, offset: u8) -> u32 {
    unsafe {
        outl(CONFIG_ADDRESS, address(bus, dev, func, offset));
        inl(CONFIG_DATA)
    }
}

pub fn write32(bus: u8, dev: u8, func: u8, offset: u8, value: u32) {
    unsafe {
        outl(CONFIG_ADDRESS, address(bus, dev, func, offset));
        outl(CONFIG_DATA, value);
    }
}

fn read8(bus: u8, dev: u8, func: u8, offset: u8) -> u8 {
    (read32(bus, dev, func, offset) >> ((offset & 3) * 8)) as u8
}

fn scan_function(bus: u8, dev: u8, func: u8, out: &mut Vec<PciDevice>) {
    let id = read32(bus, dev, func, 0x00);
    if id as u16 == 0xFFFF {
        return; // nothing here
    }
    let class_reg = read32(bus, dev, func, 0x08);
    let d = PciDevice {
        bus,
        dev,
        func,
        vendor: id as u16,
        device: (id >> 16) as u16,
        class: (class_reg >> 24) as u8,
        subclass: (class_reg >> 16) as u8,
        prog_if: (class_reg >> 8) as u8,
        irq_line: read8(bus, dev, func, 0x3C),
    };
    out.push(d);

    // A PCI-to-PCI bridge: scan the bus behind it too
    if d.class == 0x06 && d.subclass == 0x04 {
        let secondary = read8(bus, dev, func, 0x19);
        if secondary > bus {
            scan_bus(secondary, out);
        }
    }
}

fn scan_bus(bus: u8, out: &mut Vec<PciDevice>) {
    for dev in 0..32u8 {
        if read32(bus, dev, 0, 0x00) as u16 == 0xFFFF {
            continue;
        }
        let functions = if read8(bus, dev, 0, 0x0E) & 0x80 != 0 { 8 } else { 1 };
        for func in 0..functions {
            scan_function(bus, dev, func, out);
        }
    }
}

/// Walks the PCI buses and remembers every device. Needs the heap.
pub fn init() -> usize {
    let mut found = Vec::new();
    scan_bus(0, &mut found);
    let count = found.len();
    *DEVICES.lock() = found;
    count
}

pub fn devices() -> Vec<PciDevice> {
    DEVICES.lock().clone()
}

pub fn find(vendor: u16, device: u16) -> Option<PciDevice> {
    DEVICES.lock().iter().copied().find(|d| d.vendor == vendor && d.device == device)
}

/// First device of a class, for drivers that look for "any AHCI controller" etc.
#[allow(dead_code)]
pub fn find_class(class: u8, subclass: u8) -> Option<PciDevice> {
    DEVICES.lock().iter().copied().find(|d| d.class == class && d.subclass == subclass)
}

impl PciDevice {
    pub fn bar(&self, index: u8) -> Bar {
        let offset = 0x10 + index * 4;
        let v = read32(self.bus, self.dev, self.func, offset);
        if v == 0 {
            return Bar::None;
        }
        if v & 1 == 1 {
            return Bar::Io { port: (v & !0x3) as u16 };
        }
        let low = (v & !0xF) as u64;
        let base = if (v >> 1) & 3 == 2 {
            // 64-bit BAR: the upper half is in the next register
            ((read32(self.bus, self.dev, self.func, offset + 4) as u64) << 32) | low
        } else {
            low
        };
        Bar::Memory { base, prefetchable: v & 0x8 != 0 }
    }

    /// Turns on I/O decoding, memory decoding and bus mastering.
    pub fn enable(&self) {
        let cmd = read32(self.bus, self.dev, self.func, 0x04) & 0xFFFF; // keep the status half zero
        write32(self.bus, self.dev, self.func, 0x04, cmd | 0x7);
    }
}

pub fn class_name(class: u8, subclass: u8) -> &'static str {
    match (class, subclass) {
        (0x01, 0x01) => "IDE controller",
        (0x01, 0x06) => "SATA controller",
        (0x01, 0x08) => "NVMe controller",
        (0x01, _) => "storage controller",
        (0x02, _) => "network controller",
        (0x03, _) => "display controller",
        (0x04, _) => "multimedia controller",
        (0x06, 0x00) => "host bridge",
        (0x06, 0x01) => "ISA bridge",
        (0x06, 0x04) => "PCI bridge",
        (0x06, _) => "bridge",
        (0x0C, 0x03) => "USB controller",
        (0x0C, _) => "serial bus controller",
        _ => "other device",
    }
}

pub fn device_name(vendor: u16, device: u16) -> Option<&'static str> {
    match (vendor, device) {
        (0x1234, 0x1111) => Some("QEMU standard VGA"),
        (0x8086, 0x1237) => Some("Intel 440FX host bridge"),
        (0x8086, 0x7000) => Some("Intel PIIX3 ISA bridge"),
        (0x8086, 0x7010) => Some("Intel PIIX3 IDE controller"),
        (0x8086, 0x7113) => Some("Intel PIIX4 power management"),
        (0x8086, 0x100E) => Some("Intel e1000 network card"),
        (0x1B36, 0x000D) => Some("QEMU xHCI USB controller"),
        (0x1AF4, 0x1000) => Some("virtio network card"),
        (0x1AF4, 0x1001) => Some("virtio block device"),
        _ => None,
    }
}
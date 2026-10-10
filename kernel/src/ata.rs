use crate::kassert;
use crate::arch::{inb, inw, outb, outw};

#[derive(Clone, Copy)]
pub struct DriveInfo {
    pub sectors: u32,
    pub model: [u8; 40],
}

// (I/O base, control base) for the primary and secondary IDE buses
const BUSES: [(u16, u16); 2] = [(0x1F0, 0x3F6), (0x170, 0x376)];

fn bus(drive: u8) -> (u16, u16, u8) {
    let (io, ctrl) = BUSES[(drive >> 1) as usize];
    (io, ctrl, drive & 1)
}

/// ~400 ns delay: reading the control port takes about 100 ns each
fn delay(ctrl: u16) {
    for _ in 0..4 {
        unsafe { inb(ctrl) };
    }
}

fn wait_not_busy(io: u16) -> bool {
    for _ in 0..1_000_000 {
        if unsafe { inb(io + 7) } & 0x80 == 0 {
            return true;
        }
    }
    false
}

/// true = data ready, false = error or timeout
fn wait_drq(io: u16) -> bool {
    for _ in 0..1_000_000 {
        let s = unsafe { inb(io + 7) };
        if s & 0x21 != 0 {
            return false; // ERR or DF
        }
        if s & 0x80 == 0 && s & 0x08 != 0 {
            return true;
        }
    }
    false
}

pub fn identify(drive: u8) -> Option<DriveInfo> {
    kassert!(drive < 4, "invalid drive id {}", drive);
    if drive > 3 {
        return None;
    }
    let (io, ctrl, slave) = bus(drive);

    unsafe {
        outb(io + 6, 0xA0 | (slave << 4));
        delay(ctrl);
        outb(io + 2, 0);
        outb(io + 3, 0);
        outb(io + 4, 0);
        outb(io + 5, 0);
        outb(io + 7, 0xEC); // IDENTIFY

        let s = inb(io + 7);
        if s == 0 || s == 0xFF {
            return None; // no drive on this slot
        }
        if !wait_not_busy(io) {
            return None;
        }
        if inb(io + 4) != 0 || inb(io + 5) != 0 {
            return None; // ATAPI or SATA device, not plain ATA
        }
        if !wait_drq(io) {
            return None;
        }

        let mut words = [0u16; 256];
        for w in words.iter_mut() {
            *w = inw(io);
        }

        let mut model = [b' '; 40];
        for i in 0..20 {
            let w = words[27 + i]; // model string is stored byte-swapped
            model[i * 2] = (w >> 8) as u8;
            model[i * 2 + 1] = (w & 0xFF) as u8;
        }
        let sectors = words[60] as u32 | ((words[61] as u32) << 16);

        Some(DriveInfo { sectors, model })
    }
}

fn start_command(drive: u8, lba: u32, cmd: u8) -> Option<u16> {
    let (io, ctrl, slave) = bus(drive);
    if !wait_not_busy(io) {
        return None;
    }
    unsafe {
        outb(io + 6, 0xE0 | (slave << 4) | ((lba >> 24) & 0x0F) as u8);
        delay(ctrl);
        outb(io + 2, 1); // one sector
        outb(io + 3, lba as u8);
        outb(io + 4, (lba >> 8) as u8);
        outb(io + 5, (lba >> 16) as u8);
        outb(io + 7, cmd);
    }
    Some(io)
}

pub fn read_sector(drive: u8, lba: u32, buf: &mut [u8; 512]) -> bool {
    kassert!(drive < 4, "invalid drive id {}", drive);
    let Some(io) = start_command(drive, lba, 0x20) else {
        return false;
    };
    if !wait_drq(io) {
        return false;
    }
    for i in 0..256 {
        let w = unsafe { inw(io) };
        buf[i * 2] = w as u8;
        buf[i * 2 + 1] = (w >> 8) as u8;
    }
    true
}

pub fn write_sector(drive: u8, lba: u32, buf: &[u8; 512]) -> bool {
    kassert!(drive < 4, "invalid drive id {}", drive);
    let Some(io) = start_command(drive, lba, 0x30) else {
        return false;
    };
    if !wait_drq(io) {
        return false;
    }
    for i in 0..256 {
        let w = buf[i * 2] as u16 | ((buf[i * 2 + 1] as u16) << 8);
        unsafe { outw(io, w) };
    }
    unsafe { outb(io + 7, 0xE7) }; // flush write cache
    wait_not_busy(io)
}

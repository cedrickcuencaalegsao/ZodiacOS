#![allow(dead_code)]
use crate::arch::{inb, inw, outb, outw};
use crate::kassert;

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
        inb(ctrl);
    }
}

fn wait_not_busy(io: u16) -> bool {
    for _ in 0..1_000_000 {
        if inb(io + 7) & 0x80 == 0 {
            return true;
        }
    }
    false
}

/// true = data ready, false = error or timeout
fn wait_drq(io: u16) -> bool {
    for _ in 0..1_000_000 {
        let s = inb(io + 7);
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

fn start_command(drive: u8, lba: u32, cmd: u8) -> Option<u16> {
    let (io, ctrl, slave) = bus(drive);
    if !wait_not_busy(io) {
        return None;
    }

    outb(io + 6, 0xE0 | (slave << 4) | ((lba >> 24) & 0x0F) as u8);
    delay(ctrl);
    outb(io + 2, 1); // one sector
    outb(io + 3, lba as u8);
    outb(io + 4, (lba >> 8) as u8);
    outb(io + 5, (lba >> 16) as u8);
    outb(io + 7, cmd);

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
        let w = inw(io);
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
        outw(io, w);
    }
    outb(io + 7, 0xE7); // flush write cache
    wait_not_busy(io)
}

// ---- BlockDevice front end ----------------------------------------------------
use crate::base::error::{Error, Result};
use crate::base::sync::Mutex;
use alloc::{string::String, sync::Arc};

/// One ATA command at a time, whichever drive it targets: the two drives of a
/// channel share registers, so commands must not interleave.
static ATA_LOCK: Mutex<()> = Mutex::new(());

pub struct AtaDisk {
    drive: u8,
    sectors: u64,
    name: String,
}

impl super::BlockDevice for AtaDisk {
    fn name(&self) -> &str {
        &self.name
    }

    fn block_count(&self) -> u64 {
        self.sectors
    }

    fn read_blocks(&self, lba: u64, count: usize, buf: &mut [u8]) -> Result<()> {
        if buf.len() < count * 512 || lba + count as u64 > self.sectors {
            return Err(Error::InvalidArgument);
        }
        let _guard = ATA_LOCK.lock();
        for i in 0..count {
            let sector: &mut [u8; 512] = (&mut buf[i * 512..(i + 1) * 512]).try_into().unwrap();
            if !read_sector(self.drive, (lba + i as u64) as u32, sector) {
                return Err(Error::Io);
            }
        }
        Ok(())
    }

    fn write_blocks(&self, lba: u64, count: usize, data: &[u8]) -> Result<()> {
        if data.len() < count * 512 || lba + count as u64 > self.sectors {
            return Err(Error::InvalidArgument);
        }
        let _guard = ATA_LOCK.lock();
        for i in 0..count {
            let sector: &[u8; 512] = (&data[i * 512..(i + 1) * 512]).try_into().unwrap();
            if !write_sector(self.drive, (lba + i as u64) as u32, sector) {
                return Err(Error::Io);
            }
        }
        Ok(())
    }
}

/// Registers every ATA disk that answers IDENTIFY.
pub fn probe() {
    let names = ["hda", "hdb", "hdc", "hdd"];
    for drive in 0..4u8 {
        if let Some(info) = identify(drive) {
            super::register(Arc::new(AtaDisk {
                drive,
                sectors: info.sectors as u64,
                name: String::from(names[drive as usize]),
            }));
        }
    }
}

// #![allow(dead_code)]
use super::{register, MacAddr, NetDevice, NetStats};
use crate::base::error::{Error, Result};
use crate::base::sync::Mutex;
use crate::drivers::pci::{Bar, PciDevice};
use crate::{log_info, paging, pmm};
use alloc::{format, string::String, sync::Arc};
use core::sync::atomic::{fence, AtomicU64, AtomicUsize, Ordering::{Relaxed, SeqCst}};

const INTEL: u16 = 0x8086;
const MODELS: [u16; 2] = [0x100E, 0x100F]; // 82540EM (QEMU's default NIC), 82545EM

// Registers (byte offsets from BAR 0)
const CTRL: usize = 0x0000;
const STATUS: usize = 0x0008;
const EERD: usize = 0x0014;
const ICR: usize = 0x00C0;
const IMC: usize = 0x00D8;
const RCTL: usize = 0x0100;
const TCTL: usize = 0x0400;
const TIPG: usize = 0x0410;
const RDBAL: usize = 0x2800;
const RDBAH: usize = 0x2804;
const RDLEN: usize = 0x2808;
const RDH: usize = 0x2810;
const RDT: usize = 0x2818;
const TDBAL: usize = 0x3800;
const TDBAH: usize = 0x3804;
const TDLEN: usize = 0x3808;
const TDH: usize = 0x3810;
const TDT: usize = 0x3818;
const MTA: usize = 0x5200; // multicast table, 128 dwords
const RAL0: usize = 0x5400;
const RAH0: usize = 0x5404;

const CTRL_LRST: u32 = 1 << 3;
const CTRL_ASDE: u32 = 1 << 5; // auto speed detection
const CTRL_SLU: u32 = 1 << 6; // set link up
const CTRL_RST: u32 = 1 << 26;
const CTRL_PHY_RST: u32 = 1 << 31;
const RCTL_EN: u32 = 1 << 1;
const RCTL_BAM: u32 = 1 << 15; // accept broadcasts
const RCTL_SECRC: u32 = 1 << 26; // strip the CRC
const TCTL_EN: u32 = 1 << 1;
const TCTL_PSP: u32 = 1 << 3; // pad short packets
const RAH_VALID: u32 = 1 << 31;

const RING: usize = 32; // descriptors per ring
const BUF_SIZE: usize = 2048;

static COUNT: AtomicUsize = AtomicUsize::new(0);

pub fn matches(dev: &PciDevice) -> bool {
    dev.vendor == INTEL && MODELS.contains(&dev.device)
}

struct Inner {
    mmio: usize,
    rx_ring: u64,
    rx_bufs: u64,
    rx_next: usize, // next descriptor to read
    tx_ring: u64,
    tx_bufs: u64,
    tx_tail: usize, // next descriptor to fill
}

impl Inner {
    fn read(&self, reg: usize) -> u32 {
        unsafe { ((self.mmio + reg) as *const u32).read_volatile() }
    }

    fn write(&self, reg: usize, value: u32) {
        unsafe { ((self.mmio + reg) as *mut u32).write_volatile(value) };
    }

    fn eeprom(&self, word: u32) -> Result<u16> {
        self.write(EERD, 1 | (word << 8));
        for _ in 0..100_000 {
            let v = self.read(EERD);
            if v & (1 << 4) != 0 {
                return Ok((v >> 16) as u16);
            }
        }
        Err(Error::Io)
    }
}

pub struct E1000 {
    name: String,
    mac: MacAddr,
    inner: Mutex<Inner>,
    rx_packets: AtomicU64,
    tx_packets: AtomicU64,
    rx_bytes: AtomicU64,
    tx_bytes: AtomicU64,
}

pub fn probe(dev: &PciDevice) -> Result<()> {
    let base = match dev.bar(0) {
        Bar::Memory { base, .. } => base,
        _ => return Err(Error::Unsupported),
    };
    dev.enable(); // memory decoding + bus mastering (the card DMAs into our memory)
    let mmio = paging::map_mmio(base, 0x2_0000).map_err(|_| Error::Io)? as usize;

    // Rings and packet buffers live in physical memory the card reads and writes directly
    let rx_ring = pmm::alloc_zeroed().ok_or(Error::OutOfMemory)?;
    let tx_ring = pmm::alloc_zeroed().ok_or(Error::OutOfMemory)?;
    let rx_bufs = pmm::alloc_contiguous(RING * BUF_SIZE / 4096).ok_or(Error::OutOfMemory)?;
    let tx_bufs = pmm::alloc_contiguous(RING * BUF_SIZE / 4096).ok_or(Error::OutOfMemory)?;
    let nic = Inner { mmio, rx_ring, rx_bufs, rx_next: 0, tx_ring, tx_bufs, tx_tail: 0 };

    // Reset, mask every interrupt (we poll), bring the link up
    nic.write(CTRL, nic.read(CTRL) | CTRL_RST);
    for _ in 0..1_000_000 {
        if nic.read(CTRL) & CTRL_RST == 0 {
            break;
        }
    }
    crate::arch::sleep_ms(10);
    nic.write(IMC, 0xFFFF_FFFF);
    let _ = nic.read(ICR);
    nic.write(CTRL, (nic.read(CTRL) | CTRL_SLU | CTRL_ASDE) & !(CTRL_LRST | CTRL_PHY_RST));

    // MAC address: the card loads it from its EEPROM into the receive-address registers
    let ral = nic.read(RAL0);
    let rah = nic.read(RAH0);
    let mut mac: MacAddr = [ral as u8, (ral >> 8) as u8, (ral >> 16) as u8, (ral >> 24) as u8, rah as u8, (rah >> 8) as u8];
    if mac == [0; 6] {
        for i in 0..3 {
            let word = nic.eeprom(i as u32)?;
            mac[2 * i] = word as u8;
            mac[2 * i + 1] = (word >> 8) as u8;
        }
        nic.write(RAL0, u32::from_le_bytes([mac[0], mac[1], mac[2], mac[3]]));
    }
    nic.write(RAH0, mac[4] as u32 | (mac[5] as u32) << 8 | RAH_VALID);
    for i in 0..128 {
        nic.write(MTA + 4 * i, 0);
    }

    // Receive ring: every descriptor points at its own buffer
    for i in 0..RING as u64 {
        let desc = rx_ring + 16 * i;
        unsafe {
            (desc as *mut u64).write_volatile(rx_bufs + i * BUF_SIZE as u64);
            ((desc + 8) as *mut u64).write_volatile(0);
        }
    }
    nic.write(RDBAL, rx_ring as u32);
    nic.write(RDBAH, (rx_ring >> 32) as u32);
    nic.write(RDLEN, (RING * 16) as u32);
    nic.write(RDH, 0);
    nic.write(RDT, (RING - 1) as u32); // hand all but one descriptor to the card
    nic.write(RCTL, RCTL_EN | RCTL_BAM | RCTL_SECRC); // 2048-byte buffers (BSIZE = 0)

    // Transmit ring: every descriptor starts out "done" so the first use doesn't wait
    for i in 0..RING as u64 {
        let desc = tx_ring + 16 * i;
        unsafe {
            (desc as *mut u64).write_volatile(tx_bufs + i * BUF_SIZE as u64);
            ((desc + 8) as *mut u64).write_volatile(1 << 32); // status byte = DD
        }
    }
    nic.write(TDBAL, tx_ring as u32);
    nic.write(TDBAH, (tx_ring >> 32) as u32);
    nic.write(TDLEN, (RING * 16) as u32);
    nic.write(TDH, 0);
    nic.write(TDT, 0);
    nic.write(TIPG, 0x0060_200A);
    nic.write(TCTL, TCTL_EN | TCTL_PSP | (0x0F << 4) | (0x40 << 12));

    let name = format!("eth{}", COUNT.fetch_add(1, Relaxed));
    log_info!(
        "e1000: {} MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        name, mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    );
    register(Arc::new(E1000 {
        name,
        mac,
        inner: Mutex::new(nic),
        rx_packets: AtomicU64::new(0),
        tx_packets: AtomicU64::new(0),
        rx_bytes: AtomicU64::new(0),
        tx_bytes: AtomicU64::new(0),
    }));
    Ok(())
}

impl NetDevice for E1000 {
    fn name(&self) -> &str {
        &self.name
    }

    fn mac(&self) -> MacAddr {
        self.mac
    }

    fn link_up(&self) -> bool {
        self.inner.lock().read(STATUS) & 2 != 0
    }

    fn send(&self, frame: &[u8]) -> Result<()> {
        if frame.len() < 14 || frame.len() > 1514 {
            return Err(Error::InvalidArgument);
        }
        let mut nic = self.inner.lock();
        let idx = nic.tx_tail;
        let desc = nic.tx_ring + 16 * idx as u64;

        // wait until the card is done with this descriptor (the DD status bit)
        let mut spins = 0u32;
        while unsafe { ((desc + 12) as *const u8).read_volatile() } & 1 == 0 {
            spins += 1;
            if spins > 1_000_000 {
                return Err(Error::Busy);
            }
            core::hint::spin_loop();
        }

        let buffer = nic.tx_bufs + (idx * BUF_SIZE) as u64;
        unsafe {
            core::ptr::copy_nonoverlapping(frame.as_ptr(), buffer as *mut u8, frame.len());
            // length | command: end of packet (1) + insert CRC (2) + report status (8)
            ((desc + 8) as *mut u64).write_volatile(frame.len() as u64 | (0x0B << 24));
        }
        fence(SeqCst);
        nic.tx_tail = (idx + 1) % RING;
        nic.write(TDT, nic.tx_tail as u32);

        self.tx_packets.fetch_add(1, Relaxed);
        self.tx_bytes.fetch_add(frame.len() as u64, Relaxed);
        Ok(())
    }

    fn receive(&self, buf: &mut [u8]) -> Result<Option<usize>> {
        let mut nic = self.inner.lock();
        let idx = nic.rx_next;
        let desc = nic.rx_ring + 16 * idx as u64;

        let status = unsafe { ((desc + 12) as *const u8).read_volatile() };
        if status & 1 == 0 {
            return Ok(None); // the card has not filled this descriptor yet
        }
        fence(SeqCst);
        let len = unsafe { ((desc + 8) as *const u16).read_volatile() } as usize;
        let errors = unsafe { ((desc + 13) as *const u8).read_volatile() };

        // a good frame fits one buffer: end-of-packet set, no errors
        let good = status & 2 != 0 && errors == 0 && len <= buf.len();
        if good {
            let source = nic.rx_bufs + (idx * BUF_SIZE) as u64;
            unsafe { core::ptr::copy_nonoverlapping(source as *const u8, buf.as_mut_ptr(), len) };
        }

        // give the descriptor back to the card
        unsafe { ((desc + 12) as *mut u8).write_volatile(0) };
        fence(SeqCst);
        nic.rx_next = (idx + 1) % RING;
        nic.write(RDT, idx as u32);

        if !good {
            return Ok(Some(0));
        }
        self.rx_packets.fetch_add(1, Relaxed);
        self.rx_bytes.fetch_add(len as u64, Relaxed);
        Ok(Some(len))
    }

    fn stats(&self) -> NetStats {
        NetStats {
            rx_packets: self.rx_packets.load(Relaxed),
            tx_packets: self.tx_packets.load(Relaxed),
            rx_bytes: self.rx_bytes.load(Relaxed),
            tx_bytes: self.tx_bytes.load(Relaxed),
        }
    }
}
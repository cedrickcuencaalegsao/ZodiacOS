use super::framebuffer::Framebuffer;
use super::pci::{self, Bar};
use crate::arch::{inw, outw};
use crate::paging;
use core::sync::atomic::{AtomicUsize, Ordering::Relaxed};

const VBE_INDEX: u16 = 0x01CE;
const VBE_DATA: u16 = 0x01CF;

const IDX_ID: u16 = 0;
const IDX_XRES: u16 = 1;
const IDX_YRES: u16 = 2;
const IDX_BPP: u16 = 3;
const IDX_ENABLE: u16 = 4;
const IDX_X_OFFSET: u16 = 8; // which part of video memory the monitor shows
const IDX_Y_OFFSET: u16 = 9;
const IDX_VRAM_64K: u16 = 10; // video memory size in 64 KiB units

const ENABLED: u16 = 0x01;
const LINEAR_FB: u16 = 0x40;

// How many full screens fit in video memory and are usable for page flipping (1 or 2).
static PAGES: AtomicUsize = AtomicUsize::new(1);
static HEIGHT: AtomicUsize = AtomicUsize::new(0);

fn write(index: u16, value: u16) {
    outw(VBE_INDEX, index);
    outw(VBE_DATA, value);
}

fn read(index: u16) -> u16 {
    outw(VBE_INDEX, index);
    inw(VBE_DATA)
}

/// Number of screens that can be flipped between (2 = double buffering works, else 1).
pub fn pages() -> usize {
    PAGES.load(Relaxed)
}

/// Makes `page` the one the monitor shows. Drawing on the other page is invisible until
/// it is shown, so a finished frame appears all at once (no tearing).
pub fn show_page(page: usize) {
    if page < pages() {
        write(IDX_Y_OFFSET, (page * HEIGHT.load(Relaxed)) as u16);
    }
}

/// Asks the card which resolution the "monitor" prefers.
///
/// QEMU's VGA card exposes an EDID block at the start of PCI BAR 2 when it is
/// started with `-device VGA,edid=on,xres=W,yres=H`. Returns None when there is
/// no EDID (plain `-vga std`), so the caller can fall back to a default.
pub fn preferred_mode() -> Option<(u16, u16)> {
    let dev = pci::find(0x1234, 0x1111)?;
    let mmio_phys = match dev.bar(2) {
        Bar::Memory { base, .. } => base,
        _ => return None,
    };
    dev.enable();

    let base = paging::map_mmio(mmio_phys, 0x1000).ok()?;
    let mut edid = [0u8; 128];
    for (i, b) in edid.iter_mut().enumerate() {
        *b = unsafe { ((base as usize + i) as *const u8).read_volatile() };
    }
    parse_edid(&edid)
}

/// Pulls the preferred resolution (the first detailed timing) out of an EDID block.
fn parse_edid(e: &[u8; 128]) -> Option<(u16, u16)> {
    const HEADER: [u8; 8] = [0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00];
    if &e[0..8] != &HEADER[..] {
        return None;
    }
    if e.iter().fold(0u8, |sum, &b| sum.wrapping_add(b)) != 0 {
        return None; // bad checksum
    }

    // First detailed timing descriptor lives at bytes 54..72.
    let d = &e[54..72];
    if d[0] == 0 && d[1] == 0 {
        return None; // not a timing descriptor
    }
    let w = d[2] as u16 | (((d[4] & 0xF0) as u16) << 4);
    let h = d[5] as u16 | (((d[7] & 0xF0) as u16) << 4);
    if w < 640 || h < 480 {
        return None;
    }
    Some((w, h))
}

pub fn init(width: u16, height: u16) -> Result<Framebuffer, &'static str> {
    let dev = pci::find(0x1234, 0x1111).ok_or("no QEMU/Bochs VGA card on the PCI bus")?;
    let fb_phys = match dev.bar(0) {
        Bar::Memory { base, .. } => base,
        _ => return Err("the VGA card has no framebuffer memory"),
    };
    dev.enable();

    write(IDX_ID, 0xB0C5); // ask for the newest register set
    if read(IDX_ID) < 0xB0C0 {
        return Err("VBE registers not present");
    }

    write(IDX_ENABLE, 0); // the mode can only change while disabled
    write(IDX_XRES, width);
    write(IDX_YRES, height);
    write(IDX_BPP, 32);
    write(IDX_ENABLE, ENABLED | LINEAR_FB);

    let (w, h, bpp) = (read(IDX_XRES), read(IDX_YRES), read(IDX_BPP));
    if w == 0 || h == 0 || bpp != 32 {
        return Err("the VGA card rejected the mode");
    }

    let pitch = w as usize * 4;
    let frame_bytes = pitch * h as usize;

    // Double buffering: the card can show any window of its video memory, so when there is room
    // for two screens one can be drawn while the other is shown. Test it instead of trusting it:
    // move the window down one screen and see whether the card accepted that.
    write(IDX_X_OFFSET, 0);
    write(IDX_Y_OFFSET, 0);
    let vram = read(IDX_VRAM_64K) as usize * 0x10000;
    let mut pages = 1;
    if vram >= 2 * frame_bytes && 2 * (h as usize) <= u16::MAX as usize {
        write(IDX_Y_OFFSET, h);
        if read(IDX_Y_OFFSET) == h {
            pages = 2;
        }
        write(IDX_Y_OFFSET, 0);
    }
    PAGES.store(pages, Relaxed);
    HEIGHT.store(h as usize, Relaxed);

    let base = paging::map_mmio(fb_phys, (frame_bytes * pages) as u64)?;
    Ok(Framebuffer {
        base: base as usize,
        width: w as usize,
        height: h as usize,
        pitch,
    })
}

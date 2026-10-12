use super::framebuffer::Framebuffer;
use super::pci::{self, Bar};
use crate::arch::{inw, outw};
use crate::paging;

const VBE_INDEX: u16 = 0x01CE;
const VBE_DATA: u16 = 0x01CF;

const IDX_ID: u16 = 0;
const IDX_XRES: u16 = 1;
const IDX_YRES: u16 = 2;
const IDX_BPP: u16 = 3;
const IDX_ENABLE: u16 = 4;

const ENABLED: u16 = 0x01;
const LINEAR_FB: u16 = 0x40;

fn write(index: u16, value: u16) {
    unsafe {
        outw(VBE_INDEX, index);
        outw(VBE_DATA, value);
    }
}

fn read(index: u16) -> u16 {
    unsafe {
        outw(VBE_INDEX, index);
        inw(VBE_DATA)
    }
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
    let base = paging::map_mmio(fb_phys, (pitch * h as usize) as u64)?;
    Ok(Framebuffer { base: base as usize, width: w as usize, height: h as usize, pitch })
}
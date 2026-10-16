use core::arch::asm;

pub fn outb(port: u16, val: u8) {
    unsafe { asm!("out dx, al", in("dx") port, in("al") val, options(nomem, nostack)) };
}

pub fn inb(port: u16) -> u8 {
    let val: u8;
    unsafe { asm!("in al, dx", in("dx") port, out("al") val, options(nomem, nostack)) };
    val
}

pub fn outw(port: u16, val: u16) {
    unsafe { asm!("out dx, ax", in("dx") port, in("ax") val, options(nomem, nostack)) };
}

pub fn inw(port: u16) -> u16 {
    let val: u16;
    unsafe { asm!("in ax, dx", in("dx") port, out("ax") val, options(nomem, nostack)) };
    val
}

pub fn outl(port: u16, val: u32) {
    unsafe { asm!("out dx, eax", in("dx") port, in("eax") val, options(nomem, nostack)) };
}

pub fn inl(port: u16) -> u32 {
    let val: u32;
    unsafe { asm!("in eax, dx", in("dx") port, out("eax") val, options(nomem, nostack)) };
    val
}

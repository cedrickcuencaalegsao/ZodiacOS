use crate::kprintln;
use core::arch::asm;

/// Must match the layout saved by exception_vectors.S.
#[repr(C)]
pub struct TrapFrame {
    pub x: [u64; 31],
    pub elr: u64,
    pub spsr: u64,
    _pad: u64,
}

const SOURCES: [&str; 4] = [
    "current EL, SP0",
    "current EL, SPx",
    "lower EL, AArch64",
    "lower EL, AArch32",
];
const KINDS: [&str; 4] = ["synchronous", "IRQ", "FIQ", "SError"];

fn exception_class(ec: u64) -> &'static str {
    match ec {
        0x00 => "unknown reason",
        0x0E => "illegal execution state",
        0x15 => "supervisor call (SVC)",
        0x20 | 0x21 => "instruction abort",
        0x22 => "misaligned program counter",
        0x24 | 0x25 => "data abort",
        0x26 => "misaligned stack pointer",
        0x3C => "breakpoint (BRK)",
        _ => "other",
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn exception_handler_rust(vector: u64, frame: &mut TrapFrame) {
    let esr: u64;
    let far: u64;
    unsafe {
        asm!("mrs {}, esr_el1", out(reg) esr, options(nomem, nostack));
        asm!("mrs {}, far_el1", out(reg) far, options(nomem, nostack));
    }
    let ec = (esr >> 26) & 0x3F;

    // A supervisor call is not an error: report it and carry on.
    if vector % 4 == 0 && ec == 0x15 {
        kprintln!("[svc #{} handled, returning to {:#x}]", esr & 0xFFFF, frame.elr);
        return;
    }

    kprintln!("\n*** CPU EXCEPTION ***");
    kprintln!(
        "{} exception from {}",
        KINDS[(vector % 4) as usize],
        SOURCES[(vector / 4) as usize]
    );
    kprintln!("class  {:#04x} ({})", ec, exception_class(ec));
    kprintln!("esr    {:#x}", esr);
    kprintln!("elr    {:#x}  (instruction that faulted)", frame.elr);
    kprintln!("far    {:#x}  (faulting address, if any)", far);
    kprintln!("spsr   {:#x}", frame.spsr);
    super::cpu::halt()
}
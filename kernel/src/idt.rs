use crate::{kprintln, panic, slogln};
use core::arch::asm;
use core::mem::size_of;

#[repr(C)]
#[derive(Clone, Copy)]
struct Entry {
    offset_low: u16,
    selector: u16,
    ist: u8,
    attr: u8,
    offset_mid: u16,
    offset_high: u32,
    zero: u32,
}

impl Entry {
    const MISSING: Entry = Entry {
        offset_low: 0,
        selector: 0,
        ist: 0,
        attr: 0,
        offset_mid: 0,
        offset_high: 0,
        zero: 0,
    };

    fn new(handler: u64) -> Entry {
        Entry {
            offset_low: handler as u16,
            selector: 0x08, // the 64-bit code segment from the bootloader's GDT
            ist: 0,
            attr: 0x8E, // present, interrupt gate
            offset_mid: (handler >> 16) as u16,
            offset_high: (handler >> 32) as u32,
            zero: 0,
        }
    }
}

/// What the CPU pushes before calling a handler.
#[repr(C)]
struct Frame {
    rip: u64,
    cs: u64,
    rflags: u64,
    rsp: u64,
    ss: u64,
}

#[repr(C, packed)]
struct IdtPtr {
    limit: u16,
    base: u64,
}

static mut IDT: [Entry; 256] = [Entry::MISSING; 256];

fn fatal(title: &str, f: &Frame, code: Option<u64>, cr2: Option<u64>) -> ! {
    unsafe { asm!("cli") };

    slogln!(
        "[EXCEPTION] {} rip={:#x} rsp={:#x} code={:?} cr2={:?}",
        title, f.rip, f.rsp, code, cr2
    );

    panic::begin_screen(title);
    kprintln!("rip     {:#018x}", f.rip);
    kprintln!("rsp     {:#018x}", f.rsp);
    kprintln!("rflags  {:#018x}", f.rflags);
    kprintln!("cs:ss   {:#x}:{:#x}", f.cs, f.ss);
    if let Some(c) = code {
        kprintln!("error   {:#x}", c);
    }
    if let Some(a) = cr2 {
        kprintln!("address {:#018x}", a);
    }
    panic::finish()
}

macro_rules! exception {
    ($name:ident, $title:expr) => {
        extern "x86-interrupt" fn $name(f: Frame) {
            fatal($title, &f, None, None)
        }
    };
}

macro_rules! exception_with_code {
    ($name:ident, $title:expr) => {
        extern "x86-interrupt" fn $name(f: Frame, code: u64) {
            fatal($title, &f, Some(code), None)
        }
    };
}

exception!(divide_error, "DIVIDE ERROR (#DE)");
exception!(invalid_opcode, "INVALID OPCODE (#UD)");
exception_with_code!(general_protection, "GENERAL PROTECTION FAULT (#GP)");

extern "x86-interrupt" fn double_fault(f: Frame, code: u64) -> ! {
    fatal("DOUBLE FAULT (#DF)", &f, Some(code), None)
}

extern "x86-interrupt" fn page_fault(f: Frame, code: u64) {
    let cr2: u64;
    unsafe { asm!("mov {}, cr2", out(reg) cr2) };
    fatal("PAGE FAULT (#PF)", &f, Some(code), Some(cr2))
}

pub fn init() {
    unsafe {
        let idt = &raw mut IDT;
        // fn item -> *const () -> u64 (a direct fn-to-integer cast now warns)
        (*idt)[0] = Entry::new(divide_error as *const () as u64);
        (*idt)[6] = Entry::new(invalid_opcode as *const () as u64);
        (*idt)[8] = Entry::new(double_fault as *const () as u64);
        (*idt)[13] = Entry::new(general_protection as *const () as u64);
        (*idt)[14] = Entry::new(page_fault as *const () as u64);

        let ptr = IdtPtr {
            limit: (size_of::<[Entry; 256]>() - 1) as u16,
            base: idt as u64,
        };
        asm!("lidt [{}]", in(reg) &ptr, options(readonly, nostack));
    }
}
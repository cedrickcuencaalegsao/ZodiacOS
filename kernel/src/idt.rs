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
            attr: 0x8E, // present, interrupt gate (interrupts disabled on entry)
            offset_mid: (handler >> 16) as u16,
            offset_high: (handler >> 32) as u32,
            zero: 0,
        }
    }
}

/// What the CPU pushes before calling a handler.
#[repr(C)]
pub struct Frame {
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

/// Installs a handler for an interrupt vector (used by `interrupts.rs` for IRQs).
pub fn set_gate(vector: u8, handler: u64) {
    unsafe {
        let idt = &raw mut IDT;
        (*idt)[vector as usize] = Entry::new(handler);
    }
}

fn describe_page_fault(code: u64) {
    kprintln!(
        "cause   {} while {}{}",
        if code & 1 != 0 { "protection violation" } else { "page not present" },
        if code & 16 != 0 {
            "fetching an instruction"
        } else if code & 2 != 0 {
            "writing"
        } else {
            "reading"
        },
        if code & 4 != 0 { " (user mode)" } else { "" }
    );
    if code & 8 != 0 {
        kprintln!("        reserved bit set in a page table entry");
    }
}

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
    if let (Some(c), Some(a)) = (code, cr2) {
        kprintln!("address {:#018x}", a);
        describe_page_fault(c);
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
exception!(device_not_available, "DEVICE NOT AVAILABLE (#NM)");
exception_with_code!(invalid_tss, "INVALID TSS (#TS)");
exception_with_code!(segment_not_present, "SEGMENT NOT PRESENT (#NP)");
exception_with_code!(stack_segment, "STACK SEGMENT FAULT (#SS)");
exception_with_code!(general_protection, "GENERAL PROTECTION FAULT (#GP)");

extern "x86-interrupt" fn double_fault(f: Frame, code: u64) -> ! {
    fatal("DOUBLE FAULT (#DF)", &f, Some(code), None)
}

extern "x86-interrupt" fn page_fault(f: Frame, code: u64) {
    let cr2: u64;
    unsafe { asm!("mov {}, cr2", out(reg) cr2) };
    fatal("PAGE FAULT (#PF)", &f, Some(code), Some(cr2))
}

/// The one non-fatal exception: log it and keep running.
extern "x86-interrupt" fn breakpoint(f: Frame) {
    slogln!("[DEBUG] breakpoint at {:#x}", f.rip);
    kprintln!("[breakpoint at {:#x}]", f.rip);
}

pub fn init() {
    // fn item -> *const () -> u64 (a direct fn-to-integer cast warns on new nightlies)
    set_gate(0, divide_error as *const () as u64);
    set_gate(3, breakpoint as *const () as u64);
    set_gate(6, invalid_opcode as *const () as u64);
    set_gate(7, device_not_available as *const () as u64);
    set_gate(8, double_fault as *const () as u64);
    set_gate(10, invalid_tss as *const () as u64);
    set_gate(11, segment_not_present as *const () as u64);
    set_gate(12, stack_segment as *const () as u64);
    set_gate(13, general_protection as *const () as u64);
    set_gate(14, page_fault as *const () as u64);

    unsafe {
        let ptr = IdtPtr {
            limit: (size_of::<[Entry; 256]>() - 1) as u16,
            base: &raw const IDT as u64,
        };
        asm!("lidt [{}]", in(reg) &ptr, options(readonly, nostack));
    }
}
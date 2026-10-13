use core::arch::asm;
use core::cell::UnsafeCell;

pub const KERNEL_CS: u16 = 0x08;
pub const KERNEL_DS: u16 = 0x10;
pub const USER_DS: u16 = 0x18 | 3;
pub const USER_CS: u16 = 0x20 | 3;
const TSS_SELECTOR: u16 = 0x28;

const IST_STACK_SIZE: usize = 16 * 1024;

/// 64-bit TSS as 26 dwords: rsp0 = dwords 1-2, ist1 = dwords 9-10, I/O map base = dword 25.
#[repr(C, align(16))]
struct Tss([u32; 26]);

#[repr(C, align(16))]
struct IstStack([u8; IST_STACK_SIZE]);

#[repr(C, packed)]
struct GdtPointer {
    limit: u16,
    base: u64,
}

struct Shared<T>(UnsafeCell<T>);
unsafe impl<T> Sync for Shared<T> {}

static TSS: Shared<Tss> = Shared(UnsafeCell::new(Tss([0; 26])));
static GDT: Shared<[u64; 7]> = Shared(UnsafeCell::new([0; 7]));
static DOUBLE_FAULT_STACK: Shared<IstStack> = Shared(UnsafeCell::new(IstStack([0; IST_STACK_SIZE])));

pub fn init() {
    unsafe {
        let tss = &mut (*TSS.0.get()).0;
        let ist1_top = DOUBLE_FAULT_STACK.0.get() as u64 + IST_STACK_SIZE as u64;
        tss[9] = ist1_top as u32;
        tss[10] = (ist1_top >> 32) as u32;
        tss[25] = 104 << 16; // I/O permission bitmap offset beyond the TSS: no port access for ring 3

        let gdt = &mut *GDT.0.get();
        gdt[0] = 0;
        gdt[1] = 0x0020_9A00_0000_0000; // 0x08 kernel code (64-bit)
        gdt[2] = 0x0000_9200_0000_0000; // 0x10 kernel data
        gdt[3] = 0x0000_F200_0000_0000; // 0x18 user data (DPL 3)
        gdt[4] = 0x0020_FA00_0000_0000; // 0x20 user code (DPL 3, 64-bit)

        // 0x28 TSS descriptor (16 bytes)
        let base = TSS.0.get() as u64;
        let limit = 103u64;
        gdt[5] = (limit & 0xFFFF)
            | ((base & 0xFF_FFFF) << 16)
            | (0x89u64 << 40) // present, available 64-bit TSS
            | (((limit >> 16) & 0xF) << 48)
            | (((base >> 24) & 0xFF) << 56);
        gdt[6] = base >> 32;

        let pointer = GdtPointer {
            limit: (7 * 8 - 1) as u16,
            base: gdt.as_ptr() as u64,
        };
        asm!("lgdt [{}]", in(reg) &pointer, options(readonly, nostack));

        // Reload CS with a far return, then the data segment registers
        asm!(
            "push {cs}",
            "lea {tmp}, [rip + 2f]",
            "push {tmp}",
            "retfq",
            "2:",
            "mov ds, {ds:x}",
            "mov es, {ds:x}",
            "mov ss, {ds:x}",
            cs = in(reg) KERNEL_CS as u64,
            ds = in(reg) KERNEL_DS,
            tmp = out(reg) _,
        );

        asm!("ltr {0:x}", in(reg) TSS_SELECTOR, options(nomem, nostack));
    }
}

/// The stack the CPU switches to when a trap or interrupt arrives from ring 3.
/// The scheduler updates this on every context switch.
pub fn set_kernel_stack(top: u64) {
    unsafe {
        let tss = &mut (*TSS.0.get()).0;
        tss[1] = top as u32;
        tss[2] = (top >> 32) as u32;
    }
}
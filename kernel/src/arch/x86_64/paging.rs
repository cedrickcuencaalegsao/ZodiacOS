#![allow(dead_code)]

use crate::{kassert, pmm};
use core::arch::asm;
use core::sync::atomic::{AtomicU64, Ordering::Relaxed};

pub const PRESENT: u64 = 1 << 0;
pub const WRITABLE: u64 = 1 << 1;
pub const USER: u64 = 1 << 2;
pub const PCD: u64 = 1 << 4; // cache disable, for memory-mapped hardware
const HUGE: u64 = 1 << 7;
pub const NX: u64 = 1 << 63;
const ADDR_MASK: u64 = 0x000F_FFFF_FFFF_F000;

// Virtual address map (each region lives under its own top-level entry)
//   PML4[0]   kernel: all RAM identity-mapped (virtual = physical)
//   PML4[1]   user programs (the only entry that differs between address spaces)
//   PML4[136] kernel heap
//   PML4[170] task stacks (used by part 2)
pub const USER_PML4_INDEX: usize = 1;
pub const USER_BASE: u64 = 0x80_0000_0000;
pub const USER_END: u64 = 0x100_0000_0000;
pub const HEAP_START: u64 = 0x4444_4444_0000;
pub const STACK_REGION: u64 = 0x5555_0000_0000;

static KERNEL_PML4: AtomicU64 = AtomicU64::new(0);

unsafe extern "C" {
    static __text_start: u8;
    static __text_end: u8;
    static __rodata_start: u8;
    static __rodata_end: u8;
    static __kernel_end: u8;
}

/// Page table index of `virt` at `level` (3 = PML4, 2 = PDPT, 1 = PD, 0 = PT).
fn idx(virt: u64, level: u32) -> usize {
    ((virt >> (12 + 9 * level)) & 0x1FF) as usize
}

fn flush(virt: u64) {
    unsafe { asm!("invlpg [{}]", in(reg) virt, options(nostack, preserves_flags)) };
}

pub fn kernel_pml4() -> u64 {
    KERNEL_PML4.load(Relaxed)
}

pub fn current() -> u64 {
    let cr3: u64;
    unsafe { asm!("mov {}, cr3", out(reg) cr3, options(nomem, nostack)) };
    cr3 & ADDR_MASK
}

pub fn activate(pml4: u64) {
    unsafe { asm!("mov cr3, {}", in(reg) pml4, options(nostack, preserves_flags)) };
}

/// Next-level table for entry `i`, creating it if missing.
/// `user` tables get the USER bit on the way down, kernel tables never do.
unsafe fn next_table(table: *mut u64, i: usize, user: bool) -> Result<*mut u64, &'static str> {
    unsafe {
        let slot = table.add(i);
        let e = slot.read();
        if e & PRESENT != 0 {
            if e & HUGE != 0 {
                return Err("address is covered by a huge page");
            }
            if user && e & USER == 0 {
                return Err("address belongs to the kernel");
            }
            return Ok((e & ADDR_MASK) as *mut u64);
        }
        let frame = pmm::alloc_zeroed().ok_or("out of physical memory")?;
        slot.write(frame | PRESENT | WRITABLE | if user { USER } else { 0 });
        Ok(frame as *mut u64)
    }
}

/// Maps one 4 KiB page. `flags` are WRITABLE / USER / NX / PCD (PRESENT is added).
pub fn map_page(pml4: u64, virt: u64, phys: u64, flags: u64) -> Result<(), &'static str> {
    kassert!(
        virt % 4096 == 0 && phys % 4096 == 0,
        "map_page: unaligned {:#x} -> {:#x}",
        virt,
        phys
    );
    let user = flags & USER != 0;
    unsafe {
        let mut t = pml4 as *mut u64;
        for level in [3u32, 2, 1] {
            t = next_table(t, idx(virt, level), user)?;
        }
        let slot = t.add(idx(virt, 0));
        if slot.read() & PRESENT != 0 {
            return Err("page already mapped");
        }
        slot.write(phys | flags | PRESENT);
    }
    flush(virt);
    Ok(())
}

fn map_huge(pml4: u64, virt: u64, phys: u64, flags: u64) -> Result<(), &'static str> {
    let user = flags & USER != 0;
    unsafe {
        let mut t = pml4 as *mut u64;
        for level in [3u32, 2] {
            t = next_table(t, idx(virt, level), user)?;
        }
        let slot = t.add(idx(virt, 1));
        if slot.read() & PRESENT != 0 {
            return Err("already mapped");
        }
        slot.write(phys | flags | PRESENT | HUGE);
    }
    Ok(())
}

/// Maps device memory (virtual = physical), uncached. Returns the virtual address.
pub fn map_mmio(phys: u64, len: u64) -> Result<u64, &'static str> {
    const HUGE_SIZE: u64 = 2 << 20;
    let pml4 = kernel_pml4();
    let start = phys & !(HUGE_SIZE - 1);
    let end = (phys + len + HUGE_SIZE - 1) & !(HUGE_SIZE - 1);
    let mut addr = start;
    while addr < end {
        if translate(pml4, addr).is_none() {
            map_huge(pml4, addr, addr, PRESENT | WRITABLE | NX | PCD)?;
        }
        addr += HUGE_SIZE;
    }
    Ok(phys)
}

/// Removes a 4 KiB mapping and returns the physical frame it pointed at.
pub fn unmap_page(pml4: u64, virt: u64) -> Option<u64> {
    unsafe {
        let mut t = pml4 as *mut u64;
        for level in [3u32, 2, 1] {
            let e = t.add(idx(virt, level)).read();
            if e & PRESENT == 0 || e & HUGE != 0 {
                return None;
            }
            t = (e & ADDR_MASK) as *mut u64;
        }
        let slot = t.add(idx(virt, 0));
        let e = slot.read();
        if e & PRESENT == 0 {
            return None;
        }
        slot.write(0);
        flush(virt);
        Some(e & ADDR_MASK)
    }
}

/// Effective permissions of a mapping (the AND/OR of every level of the walk).
pub struct Mapping {
    pub phys: u64,
    pub writable: bool,
    pub user: bool,
    pub nx: bool,
    pub size: u64,
}

pub fn translate(pml4: u64, virt: u64) -> Option<Mapping> {
    let (mut writable, mut user, mut nx) = (true, true, false);
    let mut table = pml4 as *const u64;
    unsafe {
        for level in [3u32, 2, 1, 0] {
            let e = table.add(idx(virt, level)).read();
            if e & PRESENT == 0 {
                return None;
            }
            writable &= e & WRITABLE != 0;
            user &= e & USER != 0;
            nx |= e & NX != 0;
            let huge = (level == 1 || level == 2) && e & HUGE != 0;
            if huge || level == 0 {
                let size: u64 = match level {
                    2 => 1 << 30,
                    1 => 2 << 20,
                    _ => 4096,
                };
                let base = e & ADDR_MASK & !(size - 1);
                return Some(Mapping {
                    phys: base | (virt & (size - 1)),
                    writable,
                    user,
                    nx,
                    size,
                });
            }
            table = (e & ADDR_MASK) as *const u64;
        }
    }
    None
}

// ---- Address spaces -----------------------------------------------------------

/// A new top-level table that shares every kernel mapping but has an empty user region.
pub fn new_address_space() -> Option<u64> {
    let pml4 = pmm::alloc_zeroed()?;
    let kernel = kernel_pml4() as *const u64;
    for i in 0..512 {
        if i != USER_PML4_INDEX {
            unsafe { (pml4 as *mut u64).add(i).write(kernel.add(i).read()) };
        }
    }
    Some(pml4)
}

/// Frees the user part of an address space (tables and the frames they map) and the PML4.
pub fn destroy_address_space(pml4: u64) {
    unsafe {
        let e = (pml4 as *const u64).add(USER_PML4_INDEX).read();
        if e & PRESENT != 0 {
            free_tree(e & ADDR_MASK, 2);
        }
    }
    pmm::free_frame(pml4);
}

/// `level` is the level of `table` itself: 2 = PDPT, 1 = PD, 0 = PT (entries are data frames).
unsafe fn free_tree(table: u64, level: u32) {
    unsafe {
        let t = table as *const u64;
        for i in 0..512 {
            let e = t.add(i).read();
            if e & PRESENT == 0 {
                continue;
            }
            if level == 0 {
                pmm::free_frame(e & ADDR_MASK);
            } else {
                free_tree(e & ADDR_MASK, level - 1);
            }
        }
    }
    pmm::free_frame(table);
}

// ---- Boot-time setup ------------------------------------------------------------

fn sym(p: *const u8) -> u64 {
    p as u64
}

/// Permissions for the kernel's own pages (the first 2 MiB).
fn kernel_page_flags(addr: u64) -> u64 {
    let text = sym(&raw const __text_start)..sym(&raw const __text_end);
    let rodata = sym(&raw const __rodata_start)..sym(&raw const __rodata_end);
    if text.contains(&addr) {
        PRESENT // read + execute
    } else if rodata.contains(&addr) {
        PRESENT | NX // read only
    } else if (0xA0000..0xC0000).contains(&addr) {
        PRESENT | WRITABLE | NX | PCD // VGA memory
    } else {
        PRESENT | WRITABLE | NX // data, bss, stack, page tables
    }
}

fn enable_protection() {
    unsafe {
        // EFER.NXE: without it the NX bit in a page table entry is a reserved bit
        asm!(
            "rdmsr",
            "or eax, 0x800",
            "wrmsr",
            in("ecx") 0xC000_0080u32,
            out("eax") _,
            out("edx") _,
            options(nomem, nostack),
        );
        // CR0.WP: ring 0 must obey read-only pages too
        asm!(
            "mov {tmp}, cr0",
            "or {tmp}, 0x10000",
            "mov cr0, {tmp}",
            tmp = out(reg) _,
            options(nomem, nostack),
        );
    }
}

/// Replaces the bootloader's 2 MiB identity map with the kernel's own page tables.
pub fn init() {
    let max_phys = pmm::max_phys().min(4 << 30);
    let kernel_end = &raw const __kernel_end as u64;
    kassert!(
        kernel_end >= 0x10_0000,
        "the kernel must be linked at 1 MiB or higher"
    );

    // 4 KiB pages from the bottom of memory to the end of the kernel image, rounded
    // up to a 2 MiB boundary. Page 0 stays unmapped so null pointers fault.
    let low_end = (kernel_end + (2 << 20) - 1) & !((2 << 20) - 1);
    kassert!(
        max_phys > low_end + (2 << 20),
        "not enough RAM above the kernel image"
    );

    let pml4 = pmm::alloc_zeroed().expect("paging: out of memory");

    for page in 1..(low_end / 4096) {
        let addr = page * 4096;
        map_page(pml4, addr, addr, kernel_page_flags(addr)).expect("paging: map low memory");
    }

    // The rest of RAM with 2 MiB pages
    let mut addr = low_end;
    while addr < max_phys {
        map_huge(pml4, addr, addr, PRESENT | WRITABLE | NX).expect("paging: map RAM");
        addr += 2 << 20;
    }

    // Create the top-level entries for the heap and stack regions now, so every
    // address space created later shares them.
    unsafe {
        let top = pml4 as *mut u64;
        next_table(top, idx(HEAP_START, 3), false).expect("paging: reserve heap region");
        next_table(top, idx(STACK_REGION, 3), false).expect("paging: reserve stack region");
    }

    enable_protection();
    activate(pml4);
    KERNEL_PML4.store(pml4, Relaxed);
}

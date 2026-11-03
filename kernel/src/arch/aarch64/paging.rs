#![allow(dead_code)]
//! arch/aarch64/paging.rs
//!
//! AArch64 page tables: 4 KiB granule, 48-bit virtual addresses, 4 levels.
//!
//!   TTBR0_EL1  user half    0x0000_0000_0000_0000 ..= 0x0000_FFFF_FFFF_FFFF
//!   TTBR1_EL1  kernel half  0xFFFF_0000_0000_0000 ..= 0xFFFF_FFFF_FFFF_FFFF
//!
//! Each process owns only a TTBR0 root; the kernel half is shared, so a new address
//! space needs no copying of kernel entries (unlike x86_64's PML4 upper half).
//!
//! The boot code must already have:
//!   * set TCR_EL1 for a 4 KiB granule with T0SZ = T1SZ = 16 (48-bit VAs),
//!   * loaded MAIR_EL1 with `MAIR_VALUE` (or called `set_mair()`),
//!   * mapped the kernel and a direct map of RAM at `PHYS_OFFSET + phys` through TTBR1.
//!
//! Descriptor attribute bits used here:
//!   [1:0]  valid / table-or-page    [4:2]  AttrIndx    [7:6] AP (EL0 access, read-only)
//!   [9:8]  SH                       [10]   AF          [11]  nG
//!   [53]   PXN                      [54]   UXN
//! The hierarchical table attributes (APTable, UXNTable, ...) are never set.

use core::arch::asm;
use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{AtomicUsize, Ordering::Relaxed};

pub const PAGE_SIZE: u64 = 4096;

/// First and last (exclusive) addresses user programs may use.
pub const USER_BASE: u64 = 0x0000_0000_0040_0000;
pub const USER_END: u64 = 0x0000_7FFF_FFFF_F000;

/// Virtual address of physical address 0 in the kernel's direct map.
/// Set this to 0 only while the kernel still runs identity-mapped during early bring-up
/// (an identity-mapped kernel in TTBR0 would collide with user addresses).
pub const PHYS_OFFSET: u64 = 0xFFFF_0000_0000_0000;

// ---- descriptor bits ---------------------------------------------------------
const ADDR_MASK: u64 = 0x0000_FFFF_FFFF_F000;
const VALID: u64 = 1 << 0;
const TABLE_OR_PAGE: u64 = 1 << 1; // table at levels 0-2, page at level 3; clear = block
const ATTR_SHIFT: u64 = 2;
const ATTR_MASK: u64 = 0b111 << ATTR_SHIFT;
const AP_USER: u64 = 1 << 6; // AP[1]: EL0 may access
const AP_RO: u64 = 1 << 7; // AP[2]: read-only
const SH_OUTER: u64 = 0b10 << 8;
const SH_INNER: u64 = 0b11 << 8;
const AF: u64 = 1 << 10;
const NG: u64 = 1 << 11; // not-global: user pages
const PXN: u64 = 1 << 53;
const UXN: u64 = 1 << 54;

// ---- memory attributes (MAIR_EL1 indices) -------------------------------------
pub const ATTR_DEVICE: u64 = 0; // Device-nGnRnE
pub const ATTR_NORMAL: u64 = 1; // Normal, write-back cacheable
pub const ATTR_UNCACHED: u64 = 2; // Normal, non-cacheable (framebuffers)

/// Value for MAIR_EL1 matching the indices above.
pub const MAIR_VALUE: u64 = 0x00 | (0xFF << 8) | (0x44 << 16);

pub fn set_mair() {
    unsafe {
        asm!("msr mair_el1, {}", "isb", in(reg) MAIR_VALUE, options(nostack));
    }
}

pub fn init(){
    set_mair();
    flush_all();
}

// ---- mapping flags -------------------------------------------------------------
pub const WRITE: u32 = 1 << 0;
pub const USER: u32 = 1 << 1;
pub const EXEC: u32 = 1 << 2;
pub const DEVICE: u32 = 1 << 3;
pub const UNCACHED: u32 = 1 << 4;

/// What `translate` reports about one address.
#[derive(Clone, Copy, Debug)]
pub struct Mapping {
    /// Physical address the queried virtual address maps to (includes the page offset).
    pub phys: u64,
    pub user: bool,
    pub writable: bool,
    pub executable: bool,
    pub device: bool,
    /// Size of the mapping that was found: 4 KiB, 2 MiB or 1 GiB.
    pub size: u64,
    pub huge: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapError {
    /// Address or flags don't make sense (not page-aligned, user flag on a kernel address).
    BadAddress,
    /// Top 16 bits are neither all 0 nor all 1.
    NonCanonical,
    OutOfMemory,
    AlreadyMapped,
    /// A 2 MiB / 1 GiB block already covers this address.
    BlockConflict,
}

// ---- frame allocator hook --------------------------------------------------------
// The page-table code needs frames for new tables but doesn't own the allocator.
// Register yours once at boot, before the first map_page / new_address_space call:
//     paging::set_frame_allocator(|| frame::alloc().map(|f| f.addr()), |p| frame::free(p));
static ALLOC_FN: AtomicUsize = AtomicUsize::new(0);
static FREE_FN: AtomicUsize = AtomicUsize::new(0);

/// `alloc` returns the physical address of a free 4 KiB frame; `free` gives one back.
pub fn set_frame_allocator(alloc: fn() -> Option<u64>, free: fn(u64)) {
    ALLOC_FN.store(alloc as usize, Relaxed);
    FREE_FN.store(free as usize, Relaxed);
}

fn alloc_table() -> Option<u64> {
    let f = ALLOC_FN.load(Relaxed);
    if f == 0 {
        return None;
    }
    let f: fn() -> Option<u64> = unsafe { core::mem::transmute(f) };
    let phys = f()?;
    unsafe { core::ptr::write_bytes(phys_to_virt(phys) as *mut u8, 0, PAGE_SIZE as usize) };
    Some(phys)
}

fn free_frame(phys: u64) {
    let f = FREE_FN.load(Relaxed);
    if f != 0 {
        let f: fn(u64) = unsafe { core::mem::transmute(f) };
        f(phys);
    }
}

// ---- helpers ---------------------------------------------------------------------
#[inline]
fn phys_to_virt(phys: u64) -> u64 {
    phys + PHYS_OFFSET
}

#[inline]
fn read_entry(table: u64, idx: usize) -> u64 {
    unsafe { read_volatile((phys_to_virt(table) as *const u64).add(idx)) }
}

#[inline]
fn write_entry(table: u64, idx: usize, val: u64) {
    unsafe { write_volatile((phys_to_virt(table) as *mut u64).add(idx), val) }
}

/// Table index for `virt` at `level` (0 = top, 3 = leaf).
#[inline]
fn index(virt: u64, level: usize) -> usize {
    ((virt >> (39 - 9 * level)) & 0x1FF) as usize
}

fn read_ttbr1() -> u64 {
    let v: u64;
    unsafe { asm!("mrs {}, ttbr1_el1", out(reg) v, options(nomem, nostack)) };
    v & ADDR_MASK
}

/// Physical address of the top-level table of the address space that is active now.
pub fn current() -> u64 {
    let v: u64;
    unsafe { asm!("mrs {}, ttbr0_el1", out(reg) v, options(nomem, nostack)) };
    v & ADDR_MASK
}

/// Root of the shared kernel half.
pub fn kernel_root() -> u64 {
    read_ttbr1()
}

/// Which root governs `virt`: user addresses use `root`, kernel addresses always TTBR1.
fn root_for(root: u64, virt: u64) -> Option<u64> {
    match virt >> 48 {
        0 => Some(root & ADDR_MASK),
        0xFFFF => Some(read_ttbr1()),
        _ => None,
    }
}

/// Builds a level-3 page descriptor.
fn leaf_descriptor(phys: u64, flags: u32) -> u64 {
    let attr = if flags & DEVICE != 0 {
        ATTR_DEVICE
    } else if flags & UNCACHED != 0 {
        ATTR_UNCACHED
    } else {
        ATTR_NORMAL
    };
    let mut d = (phys & ADDR_MASK) | VALID | TABLE_OR_PAGE | AF | (attr << ATTR_SHIFT);
    d |= if attr == ATTR_DEVICE { SH_OUTER } else { SH_INNER };

    let user = flags & USER != 0;
    if user {
        d |= AP_USER | NG;
    }
    if flags & WRITE == 0 {
        d |= AP_RO;
    }
    // Never execute user pages at EL1, never execute kernel pages at EL0, and
    // nothing is executable unless asked for.
    if flags & EXEC == 0 {
        d |= PXN | UXN;
    } else if user {
        d |= PXN;
    } else {
        d |= UXN;
    }
    d
}

fn mapping_from(desc: u64, virt: u64, size: u64) -> Mapping {
    let base = desc & ADDR_MASK & !(size - 1);
    let user = desc & AP_USER != 0;
    Mapping {
        phys: base | (virt & (size - 1)),
        user,
        writable: desc & AP_RO == 0,
        executable: if user { desc & UXN == 0 } else { desc & PXN == 0 },
        device: (desc & ATTR_MASK) >> ATTR_SHIFT == ATTR_DEVICE,
        size,
        huge: size != PAGE_SIZE,
    }
}

// ---- TLB -------------------------------------------------------------------------
/// Drops any cached translation for the page containing `virt` (all ASIDs, all cores).
pub fn flush_page(virt: u64) {
    unsafe {
        asm!(
            "dsb ishst",
            "tlbi vaae1is, {}",
            "dsb ish",
            "isb",
            in(reg) virt >> 12,
            options(nostack)
        );
    }
}

pub fn flush_all() {
    unsafe {
        asm!("dsb ishst", "tlbi vmalle1is", "dsb ish", "isb", options(nostack));
    }
}

// ---- public API ------------------------------------------------------------------

/// Walks the tables rooted at `root` and describes how `virt` is mapped, or None if it
/// isn't. Kernel-half addresses are looked up through TTBR1 whatever `root` is.
pub fn translate(root: u64, virt: u64) -> Option<Mapping> {
    let mut table = root_for(root, virt)?;
    for level in 0..4 {
        let desc = read_entry(table, index(virt, level));
        if desc & VALID == 0 {
            return None;
        }
        let is_table = desc & TABLE_OR_PAGE != 0;
        match level {
            3 => {
                // at level 3 bit 1 must be set (page); clear is reserved/invalid
                return if is_table {
                    Some(mapping_from(desc, virt, PAGE_SIZE))
                } else {
                    None
                };
            }
            1 | 2 if !is_table => {
                let size = if level == 1 { 1 << 30 } else { 1 << 21 };
                return Some(mapping_from(desc, virt, size));
            }
            0 if !is_table => return None, // no level-0 blocks with a 4 KiB granule
            _ => table = desc & ADDR_MASK,
        }
    }
    None
}

/// Maps one 4 KiB page. Intermediate tables are allocated as needed.
/// Kernel-half addresses are mapped in the shared TTBR1 tables (visible to everyone).
pub fn map_page(root: u64, virt: u64, phys: u64, flags: u32) -> Result<(), MapError> {
    if virt & (PAGE_SIZE - 1) != 0 || phys & (PAGE_SIZE - 1) != 0 {
        return Err(MapError::BadAddress);
    }
    let mut table = root_for(root, virt).ok_or(MapError::NonCanonical)?;
    if flags & USER != 0 && virt >> 48 != 0 {
        return Err(MapError::BadAddress);
    }

    for level in 0..3 {
        let idx = index(virt, level);
        let desc = read_entry(table, idx);
        if desc & VALID == 0 {
            let new = alloc_table().ok_or(MapError::OutOfMemory)?;
            write_entry(table, idx, new | VALID | TABLE_OR_PAGE);
            table = new;
        } else if desc & TABLE_OR_PAGE == 0 {
            return Err(MapError::BlockConflict);
        } else {
            table = desc & ADDR_MASK;
        }
    }

    let idx = index(virt, 3);
    if read_entry(table, idx) & VALID != 0 {
        return Err(MapError::AlreadyMapped);
    }
    write_entry(table, idx, leaf_descriptor(phys, flags));
    // make the new entries visible to the table walker before the next access
    unsafe { asm!("dsb ishst", "isb", options(nostack)) };
    Ok(())
}

/// Maps `len` bytes starting at `virt` -> `phys` (both page-aligned; `len` rounds up).
/// On failure, pages already mapped by this call are unmapped again.
pub fn map_range(root: u64, virt: u64, phys: u64, len: u64, flags: u32) -> Result<(), MapError> {
    let pages = (len + PAGE_SIZE - 1) / PAGE_SIZE;
    for i in 0..pages {
        if let Err(e) = map_page(root, virt + i * PAGE_SIZE, phys + i * PAGE_SIZE, flags) {
            for j in 0..i {
                unmap_page(root, virt + j * PAGE_SIZE);
            }
            return Err(e);
        }
    }
    Ok(())
}

/// Removes the mapping of one page and returns the physical frame it pointed to.
/// Empty intermediate tables are left in place (they are freed with the address space).
pub fn unmap_page(root: u64, virt: u64) -> Option<u64> {
    let mut table = root_for(root, virt)?;
    for level in 0..3 {
        let desc = read_entry(table, index(virt, level));
        if desc & VALID == 0 || desc & TABLE_OR_PAGE == 0 {
            return None; // not mapped, or covered by a block
        }
        table = desc & ADDR_MASK;
    }
    let idx = index(virt, 3);
    let desc = read_entry(table, idx);
    if desc & VALID == 0 {
        return None;
    }
    write_entry(table, idx, 0);
    flush_page(virt);
    Some(desc & ADDR_MASK)
}

/// Creates an empty user address space and returns its root (physical address).
/// The kernel half needs nothing: it lives in TTBR1.
pub fn new_address_space() -> Option<u64> {
    alloc_table()
}

/// Frees an address space's page tables. With `free_leaves` the user frames they map are
/// freed too; pass false if some of them are shared or device memory.
/// Never call this on the address space that is currently active.
pub fn destroy_address_space(root: u64, free_leaves: bool) {
    if root == current() {
        return;
    }
    free_table(root & ADDR_MASK, 0, free_leaves);
}

fn free_table(table: u64, level: usize, free_leaves: bool) {
    for i in 0..512 {
        let d = read_entry(table, i);
        if d & VALID == 0 {
            continue;
        }
        if level == 3 {
            if free_leaves {
                free_frame(d & ADDR_MASK);
            }
        } else if d & TABLE_OR_PAGE != 0 {
            free_table(d & ADDR_MASK, level + 1, free_leaves);
        }
        // blocks are not owned by this address space: leave them alone
    }
    free_frame(table);
}

/// Makes `root` the active user address space.
/// ASID 0 is used for everything, so the user TLB entries are flushed on every switch.
pub fn switch_to(root: u64) {
    unsafe {
        asm!(
            "msr ttbr0_el1, {}",
            "isb",
            "tlbi vmalle1is",
            "dsb ish",
            "isb",
            in(reg) root & ADDR_MASK,
            options(nostack)
        );
    }
}
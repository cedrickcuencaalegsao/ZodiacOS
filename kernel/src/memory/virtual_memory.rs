// kernel/src/memory/virtual_memory.rs
//! x86_64 4-level paging: page tables, per-process address spaces, and the
//! kernel's own mapping with per-section permissions.
use super::{
    layout::*,
    physical,
    security::{MapError, Perm},
};
use crate::sync::IrqSpinLock;
use core::arch::asm;
use core::mem::ManuallyDrop;
use core::sync::atomic::{AtomicU64, Ordering::Relaxed};
use shared::kassert;

pub const PAGE_SIZE: u64 = 4096;
pub const HUGE_SIZE: u64 = 2 * 1024 * 1024;

// ---- Virtual address layout (each PML4 slot covers 512 GiB) -----------------------------
//   slot 0        kernel identity map of the first 4 GiB (shared, supervisor-only)
//   slots 1..128  user space (private per process)
//   slot 136      kernel heap (shared)
//   slot 160      MMIO window (shared)
//   slot 176      kernel stacks (shared)
pub const USER_BASE: u64 = 0x0000_0080_0000_0000;
pub const USER_LIMIT: u64 = 0x0000_4000_0000_0000;
pub const MMIO_BASE: u64 = 0x0000_5000_0000_0000;
const MMIO_END: u64 = MMIO_BASE + (1 << 30); // 1 GiB of address space
pub const STACK_REGION: u64 = 0x0000_5800_0000_0000;

static MMIO_NEXT: AtomicU64 = AtomicU64::new(MMIO_BASE);
static RAM_TOP: AtomicU64 = AtomicU64::new(0);
static KERNEL_PML4: AtomicU64 = AtomicU64::new(0);

/// Physical memory is identity-mapped for now. Change this one constant
/// when you move to a higher-half kernel with a direct map.
pub const PHYS_OFFSET: u64 = 0;
pub fn phys_to_virt(p: u64) -> u64 {
    p + PHYS_OFFSET
}

pub const PRESENT: u64 = 1 << 0;
pub const WRITABLE: u64 = 1 << 1;
pub const USER: u64 = 1 << 2;
pub const PWT: u64 = 1 << 3;
pub const PCD: u64 = 1 << 4;
pub const HUGE: u64 = 1 << 7;
pub const NX: u64 = 1 << 63;
const ADDR_MASK: u64 = 0x000f_ffff_ffff_f000;
const HUGE_ADDR_MASK: u64 = 0x000f_ffff_ffe0_0000;

type Table = [u64; 512];

/// PML4 slots that belong to a process (everything else is shared with the kernel).
fn is_user_slot(i: usize) -> bool {
    let i = i as u64;
    i >= USER_BASE >> 39 && i < USER_LIMIT >> 39
}

unsafe fn table(phys: u64) -> &'static mut Table {
    unsafe { &mut *(phys_to_virt(phys) as *mut Table) }
}

fn idx(virt: u64, level: u32) -> usize {
    ((virt >> (12 + 9 * level)) & 0x1ff) as usize
}

fn flush(virt: u64) {
    unsafe {
        asm!("invlpg [{}]", in(reg) virt, options(nostack, preserves_flags));
    }
}

fn inter_flags(p: Perm) -> u64 {
    PRESENT | WRITABLE | if p.user { USER } else { 0 }
}

/// Physical address of the active PML4 (CR3).
pub fn current() -> u64 {
    let cr3: u64;
    unsafe {
        asm!("mov {}, cr3", out(reg) cr3, options(nomem, nostack, preserves_flags));
    }
    cr3 & ADDR_MASK
}

/// Switch to the page tables rooted at `pml4`.
pub unsafe fn load_cr3(pml4: u64) {
    unsafe {
        asm!("mov cr3, {}", in(reg) pml4, options(nostack));
    }
}

unsafe fn next_table(entry: *mut u64, inter: u64) -> Result<&'static mut Table, MapError> {
    unsafe {
        if *entry & PRESENT == 0 {
            let f = physical::alloc_zeroed().ok_or(MapError::OutOfFrames)?;
            *entry = f | inter;
        } else {
            *entry |= inter & USER;
        }
        Ok(table(*entry & ADDR_MASK))
    }
}

/// Turn a 2 MiB mapping into 512 4 KiB mappings with identical flags.
unsafe fn split_huge(pde: *mut u64) -> Result<(), MapError> {
    unsafe {
        let old = *pde;
        let base = old & HUGE_ADDR_MASK;
        let flags = old & !HUGE_ADDR_MASK & !HUGE & !(1 << 12); // bit 12 = PAT in huge entries
        let pt_frame = physical::alloc_zeroed().ok_or(MapError::OutOfFrames)?;
        let pt = table(pt_frame);
        for i in 0..512 {
            pt[i] = (base + i as u64 * PAGE_SIZE) | flags;
        }
        *pde = pt_frame | PRESENT | WRITABLE | (old & USER);
        Ok(())
    }
}

/// What a virtual address resolves to, with the *effective* permissions
/// (writable/user must hold at every level, nx at any level).
#[derive(Clone, Copy, Debug)]
pub struct Mapping {
    pub phys: u64, // physical address that `virt` maps to (includes the page offset)
    pub size: u64, // page size in bytes (4 KiB, 2 MiB or 1 GiB)
    pub writable: bool,
    pub user: bool,
    pub nx: bool,
}

/// Walks the tables rooted at `pml4`. Works on any address space, active or not.
pub fn translate(pml4: u64, virt: u64) -> Option<Mapping> {
    unsafe {
        let (mut writable, mut user, mut nx) = (true, true, false);
        let mut tbl = table(pml4);
        for level in (0..=3u32).rev() {
            let e = tbl[idx(virt, level)];
            if e & PRESENT == 0 {
                return None;
            }
            writable &= e & WRITABLE != 0;
            user &= e & USER != 0;
            nx |= e & NX != 0;
            if level == 0 || (level <= 2 && e & HUGE != 0) {
                let size = 1u64 << (12 + 9 * level);
                let phys = (e & !(size - 1) & ADDR_MASK) | (virt & (size - 1));
                return Some(Mapping { phys, size, writable, user, nx });
            }
            tbl = table(e & ADDR_MASK);
        }
        None
    }
}

/// Frees a page table and everything below it. Leaf (4 KiB) frames are freed too,
/// because frames mapped into a user space are owned by that space.
unsafe fn free_table(phys: u64, level: u32) {
    unsafe {
        let t = table(phys);
        for i in 0..512 {
            let e = t[i];
            if e & PRESENT == 0 {
                continue;
            }
            if level == 0 {
                physical::free_frame(e & ADDR_MASK);
            } else if e & HUGE == 0 {
                free_table(e & ADDR_MASK, level - 1);
            }
            // huge leaf pages are never created for user spaces: nothing to free
        }
        physical::free_frame(phys);
    }
}

pub struct AddressSpace {
    pml4: u64,
}

impl AddressSpace {
    /// An empty address space (nothing mapped, not even the kernel).
    pub fn new() -> Result<Self, MapError> {
        Ok(Self {
            pml4: physical::alloc_zeroed().ok_or(MapError::OutOfFrames)?,
        })
    }

    /// A process address space: the kernel's slots are shared (supervisor-only),
    /// the user slots are private and start empty. Kernel slots that are created
    /// *after* this call are not visible here, so every kernel region reserves
    /// its slot in `init()`.
    pub fn new_user() -> Result<Self, MapError> {
        let kernel = KERNEL_PML4.load(Relaxed);
        kassert!(kernel != 0, "virtual memory is not initialised");
        let space = Self::new()?;
        unsafe {
            let src = table(kernel);
            let dst = table(space.pml4);
            for i in 0..512 {
                if !is_user_slot(i) && src[i] & PRESENT != 0 {
                    dst[i] = src[i];
                }
            }
        }
        Ok(space)
    }

    /// Hands the address space over as a raw PML4 address (no Drop runs).
    pub fn into_raw(self) -> u64 {
        let p = self.pml4;
        core::mem::forget(self);
        p
    }

    /// Rebuilds an address space from `into_raw`. The caller owns it again.
    pub unsafe fn from_raw(pml4: u64) -> Self {
        Self { pml4 }
    }

    /// Physical address of the PML4, for `translate`.
    pub fn root(&self) -> u64 {
        self.pml4
    }

    /// Makes sure the PML4 slot for `virt` exists, so address spaces created later
    /// inherit it.
    pub fn reserve_slot(&mut self, virt: u64) -> Result<(), MapError> {
        unsafe {
            next_table(&mut table(self.pml4)[idx(virt, 3)], PRESENT | WRITABLE)?;
        }
        Ok(())
    }

    /// Walks (and creates) the tables down to the 4 KiB entry for `virt`.
    unsafe fn leaf(&mut self, virt: u64, inter: u64) -> Result<*mut u64, MapError> {
        unsafe {
            let pml4 = table(self.pml4);
            let pdpt = next_table(&mut pml4[idx(virt, 3)], inter)?;
            let pd = next_table(&mut pdpt[idx(virt, 2)], inter)?;
            let pde: *mut u64 = &mut pd[idx(virt, 1)];
            if *pde & PRESENT != 0 && *pde & HUGE != 0 {
                split_huge(pde)?;
            }
            let pt = next_table(pde, inter)?;
            Ok(&mut pt[idx(virt, 0)] as *mut u64)
        }
    }

    pub fn map_2m(&mut self, virt: u64, phys: u64, perm: Perm) -> Result<(), MapError> {
        if virt % HUGE_SIZE != 0 || phys % HUGE_SIZE != 0 {
            return Err(MapError::Misaligned);
        }
        let flags = perm.leaf_flags()?;
        unsafe {
            let inter = inter_flags(perm);
            let pml4 = table(self.pml4);
            let pdpt = next_table(&mut pml4[idx(virt, 3)], inter)?;
            let pd = next_table(&mut pdpt[idx(virt, 2)], inter)?;
            let e = &mut pd[idx(virt, 1)];
            if *e & PRESENT != 0 {
                return Err(MapError::AlreadyMapped);
            }
            *e = phys | flags | HUGE;
        }
        Ok(())
    }

    pub fn map_4k(&mut self, virt: u64, phys: u64, perm: Perm) -> Result<(), MapError> {
        if virt % PAGE_SIZE != 0 || phys % PAGE_SIZE != 0 {
            return Err(MapError::Misaligned);
        }
        // User pages only go in the user range, so they can never land in a
        // page table that is shared with the kernel.
        if perm.user && !(USER_BASE..USER_LIMIT).contains(&virt) {
            return Err(MapError::OutOfRange);
        }
        let flags = perm.leaf_flags()?;
        unsafe {
            let e = self.leaf(virt, inter_flags(perm))?;
            if *e & PRESENT != 0 {
                return Err(MapError::AlreadyMapped);
            }
            *e = phys | flags;
        }
        flush(virt);
        Ok(())
    }

    /// Change permissions of an existing page (splits a 2 MiB page if needed).
    pub fn protect_4k(&mut self, virt: u64, perm: Perm) -> Result<(), MapError> {
        let flags = perm.leaf_flags()?;
        unsafe {
            let e = self.leaf(virt, inter_flags(perm))?;
            if *e & PRESENT == 0 {
                return Err(MapError::NotMapped);
            }
            *e = (*e & ADDR_MASK) | flags;
        }
        flush(virt);
        Ok(())
    }

    /// Unmap a page and return its physical address (use for guard pages).
    pub fn unmap_4k(&mut self, virt: u64) -> Result<u64, MapError> {
        unsafe {
            let e = self.leaf(virt, PRESENT | WRITABLE)?;
            if *e & PRESENT == 0 {
                return Err(MapError::NotMapped);
            }
            let phys = *e & ADDR_MASK;
            *e = 0;
            flush(virt);
            Ok(phys)
        }
    }

    pub unsafe fn activate(&self) {
        unsafe { load_cr3(self.pml4) }
    }
}

/// Dropping an address space frees its page tables and every frame mapped into
/// its private (user) part. Never drop the one that is currently active.
impl Drop for AddressSpace {
    fn drop(&mut self) {
        kassert!(current() != self.pml4, "dropping the active address space");
        unsafe {
            let pml4 = table(self.pml4);
            for i in 0..512 {
                if is_user_slot(i) && pml4[i] & PRESENT != 0 {
                    free_table(pml4[i] & ADDR_MASK, 2);
                }
            }
        }
        physical::free_frame(self.pml4);
    }
}

// ---- Raw-handle API for code that stores a PML4 address (process, scheduler) -----------

/// The kernel's own PML4. Kernel threads run on this one.
pub fn kernel_pml4() -> u64 {
    KERNEL_PML4.load(Relaxed)
}

/// Switch to `pml4` (skips the CR3 write, and the TLB flush, when it is already active).
pub fn activate(pml4: u64) {
    if current() != pml4 {
        unsafe { load_cr3(pml4) };
    }
}

/// A new process address space. Returns its PML4 address.
pub fn new_address_space() -> Option<u64> {
    AddressSpace::new_user().ok().map(AddressSpace::into_raw)
}

/// Frees an address space made by `new_address_space`, including all frames mapped into it.
pub fn destroy_address_space(pml4: u64) {
    kassert!(pml4 != kernel_pml4(), "cannot destroy the kernel address space");
    drop(unsafe { AddressSpace::from_raw(pml4) });
}

/// Maps one page into a process address space (W^X is enforced by `Perm`).
pub fn map_page(pml4: u64, virt: u64, phys: u64, perm: Perm) -> Result<(), MapError> {
    kassert!(pml4 != kernel_pml4(), "use kernel_map for kernel mappings");
    let mut space = ManuallyDrop::new(unsafe { AddressSpace::from_raw(pml4) });
    space.map_4k(virt, phys, perm)
}

/// Maps one page into the kernel's address space (kernel stacks, ...).
pub fn kernel_map(virt: u64, phys: u64, perm: Perm) -> Result<(), MapError> {
    let mut guard = KERNEL_SPACE.lock();
    let space = guard.as_mut().ok_or(MapError::NotMapped)?;
    space.map_4k(virt, phys, perm)
}

/// Unmaps one kernel page and returns the frame that was behind it.
pub fn kernel_unmap(virt: u64) -> Option<u64> {
    let mut guard = KERNEL_SPACE.lock();
    guard.as_mut()?.unmap_4k(virt).ok()
}

pub static KERNEL_SPACE: IrqSpinLock<Option<AddressSpace>> = IrqSpinLock::new(None);

pub fn init(ram_end: u64) {
    let mut space = AddressSpace::new().expect("pml4 frame");
    let ram_end = align_up(ram_end, HUGE_SIZE);
    RAM_TOP.store(ram_end, Relaxed);

    // Identity-map the first 4 GiB with 2 MiB pages:
    // RAM is RW+NX; everything above RAM (PCI BARs, APIC...) is uncached RW+NX.
    let mut a = 0u64;
    while a < (4u64 << 30) {
        let perm = if a < ram_end {
            Perm::KERNEL_RW
        } else {
            Perm::KERNEL_MMIO
        };
        space.map_2m(a, a, perm).expect("identity map");
        a += HUGE_SIZE;
    }

    // Per-section permissions for the kernel image (4 KiB granularity).
    protect_range(&mut space, text_start(), text_end(), Perm::KERNEL_RX);
    protect_range(&mut space, rodata_start(), rodata_end(), Perm::KERNEL_RO);
    protect_range(&mut space, data_start(), kernel_end(), Perm::KERNEL_RW);

    // Create the PML4 slots for the kernel regions now, so every process address
    // space created later shares them.
    space.reserve_slot(super::heap::HEAP_START).expect("heap slot");
    space.reserve_slot(MMIO_BASE).expect("mmio slot");
    space.reserve_slot(STACK_REGION).expect("stack slot");

    KERNEL_PML4.store(space.root(), Relaxed);
    unsafe {
        space.activate();
    }
    *KERNEL_SPACE.lock() = Some(space);
}

fn protect_range(space: &mut AddressSpace, start: u64, end: u64, perm: Perm) {
    let mut p = align_down(start, PAGE_SIZE);
    let end = align_up(end, PAGE_SIZE);
    while p < end {
        space.protect_4k(p, perm).expect("protect kernel section");
        p += PAGE_SIZE;
    }
}

/// Maps device memory (framebuffer, BARs, ...) as uncached, kernel-only, non-executable.
/// Returns the virtual address that corresponds to `phys`.
pub fn map_mmio(phys: u64, len: u64) -> Result<u64, &'static str> {
    if len == 0 {
        return Err("map_mmio: empty range");
    }
    let end = phys.checked_add(len).ok_or("map_mmio: range overflows")?;

    // Already covered by the identity map (and uncached, because it is above RAM).
    if phys >= RAM_TOP.load(Relaxed) && end <= (4u64 << 30) {
        return Ok(phys);
    }

    let start = align_down(phys, PAGE_SIZE);
    let bytes = align_up(end, PAGE_SIZE) - start;
    let virt = MMIO_NEXT.fetch_add(bytes, Relaxed);
    if virt + bytes > MMIO_END {
        return Err("map_mmio: out of MMIO address space");
    }

    let mut guard = KERNEL_SPACE.lock();
    let space = guard.as_mut().ok_or("map_mmio: virtual memory not initialised")?;
    for off in (0..bytes).step_by(PAGE_SIZE as usize) {
        space
            .map_4k(virt + off, start + off, Perm::KERNEL_MMIO)
            .map_err(|_| "map_mmio: cannot map page")?;
    }
    Ok(virt + (phys - start))
}
use crate::kassert;
use crate::sync::SpinLock;
use core::ptr;

pub const FRAME_SIZE: u64 = 4096;

const MEMMAP_ADDR: usize = 0x4000;      // written by boot.asm
const MAX_REGIONS: usize = 32;
const BITMAP_ADDR: usize = 0x10_0000;   // 1 MiB, inside the boot identity map
const RESERVED_BELOW: usize = 0x10_0000; // BIOS data, kernel image, stack, VGA: never handed out
const MAX_FRAMES: usize = 1 << 20;      // tracks up to 4 GiB of RAM

/// One BIOS memory map entry (INT 0x15 E820 layout).
#[repr(C)]
#[derive(Clone, Copy)]
#[allow(dead_code)]
pub struct Region {
    pub base: u64,
    pub len: u64,
    pub kind: u32,
    attrs: u32,
}

impl Region {
    const EMPTY: Region = Region { base: 0, len: 0, kind: 0, attrs: 0 };

    pub fn kind_name(&self) -> &'static str {
        match self.kind {
            1 => "usable",
            2 => "reserved",
            3 => "ACPI reclaimable",
            4 => "ACPI NVS",
            5 => "bad memory",
            _ => "unknown",
        }
    }
}

struct Pmm {
    bitmap: *mut u8, // bit set = frame in use
    frames: usize,
    total: usize,    // allocatable frames after init
    free: usize,
    hint: usize,     // where the next search starts
    max_phys: u64,
    regions: [Region; MAX_REGIONS],
    nregions: usize,
}

unsafe impl Send for Pmm {}

static PMM: SpinLock<Pmm> = SpinLock::new(Pmm {
    bitmap: ptr::null_mut(),
    frames: 0,
    total: 0,
    free: 0,
    hint: 0,
    max_phys: 0,
    regions: [Region::EMPTY; MAX_REGIONS],
    nregions: 0,
});

impl Pmm {
    fn test(&self, i: usize) -> bool {
        unsafe { *self.bitmap.add(i / 8) & (1 << (i % 8)) != 0 }
    }

    fn set(&mut self, i: usize) {
        unsafe { *self.bitmap.add(i / 8) |= 1 << (i % 8) };
    }

    fn clear(&mut self, i: usize) {
        unsafe { *self.bitmap.add(i / 8) &= !(1 << (i % 8)) };
    }

    /// First run of `count` free frames inside [from, to).
    fn scan(&self, from: usize, to: usize, count: usize) -> Option<usize> {
        let (mut run, mut start, mut i) = (0, 0, from);
        while i < to {
            // skip eight fully used frames at a time
            if run == 0 && i % 8 == 0 && i + 8 <= to && unsafe { *self.bitmap.add(i / 8) } == 0xFF {
                i += 8;
                continue;
            }
            if self.test(i) {
                run = 0;
            } else {
                if run == 0 {
                    start = i;
                }
                run += 1;
                if run == count {
                    return Some(start);
                }
            }
            i += 1;
        }
        None
    }

    fn alloc(&mut self, count: usize) -> Option<usize> {
        if count == 0 || self.free < count {
            return None;
        }
        let found = if count == 1 {
            self.scan(self.hint, self.frames, 1).or_else(|| self.scan(0, self.hint, 1))
        } else {
            self.scan(0, self.frames, count)
        };
        let found = found?;
        for k in 0..count {
            self.set(found + k);
        }
        self.free -= count;
        self.hint = found + count;
        Some(found)
    }
}

fn reserve(p: &mut Pmm, from: usize, to: usize) {
    for f in from..to {
        if f < p.frames && !p.test(f) {
            p.set(f);
            p.free -= 1;
        }
    }
}

/// Reads the BIOS memory map and builds the frame bitmap.
pub fn init() {
    let count = unsafe { ptr::read_volatile(MEMMAP_ADDR as *const u32) } as usize;
    kassert!(
        count > 0 && count <= MAX_REGIONS,
        "no BIOS memory map (count = {}), rebuild with the new boot.asm",
        count
    );

    let mut regions = [Region::EMPTY; MAX_REGIONS];
    let mut max_phys = 0u64;
    for i in 0..count {
        let r = unsafe { ptr::read_unaligned((MEMMAP_ADDR + 16 + i * 24) as *const Region) };
        if r.kind == 1 {
            max_phys = max_phys.max(r.base + r.len);
        }
        regions[i] = r;
    }

    let frames = ((max_phys / FRAME_SIZE) as usize).min(MAX_FRAMES);
    kassert!(frames > RESERVED_BELOW / 4096 + 16, "not enough RAM ({} frames)", frames);

    let bytes = (frames + 7) / 8;
    let bitmap = BITMAP_ADDR as *mut u8;
    unsafe { ptr::write_bytes(bitmap, 0xFF, bytes) }; // everything starts out "in use"

    let mut p = PMM.lock();
    p.bitmap = bitmap;
    p.frames = frames;
    p.free = 0;
    p.hint = 0;
    p.max_phys = max_phys;
    p.regions = regions;
    p.nregions = count;

    // Free every frame that lies fully inside a usable region
    for r in regions.iter().take(count).filter(|r| r.kind == 1) {
        let first = ((r.base + FRAME_SIZE - 1) / FRAME_SIZE) as usize;
        let last = (((r.base + r.len) / FRAME_SIZE) as usize).min(frames);
        for f in first..last {
            if p.test(f) {
                p.clear(f);
                p.free += 1;
            }
        }
    }

    // Keep the first MiB and the bitmap itself out of circulation
    reserve(&mut p, 0, RESERVED_BELOW / 4096);
    let bitmap_frames = (bytes + 4095) / 4096;
    reserve(&mut p, BITMAP_ADDR / 4096, BITMAP_ADDR / 4096 + bitmap_frames);
    p.total = p.free;
}

pub fn alloc_frame() -> Option<u64> {
    alloc_contiguous(1)
}

/// `count` physically contiguous frames. Returns the address of the first one.
pub fn alloc_contiguous(count: usize) -> Option<u64> {
    let mut p = PMM.lock();
    p.alloc(count).map(|f| f as u64 * FRAME_SIZE)
}

pub fn alloc_zeroed() -> Option<u64> {
    let addr = alloc_frame()?;
    unsafe { ptr::write_bytes(addr as *mut u8, 0, FRAME_SIZE as usize) };
    Some(addr)
}

pub fn free_frame(addr: u64) {
    kassert!(addr % FRAME_SIZE == 0, "free_frame: {:#x} is not frame aligned", addr);
    let i = (addr / FRAME_SIZE) as usize;
    let mut p = PMM.lock();
    kassert!(i < p.frames, "free_frame: {:#x} is outside managed memory", addr);
    kassert!(i >= RESERVED_BELOW / 4096, "free_frame: {:#x} is reserved memory", addr);
    kassert!(p.test(i), "double free of frame {:#x}", addr);
    p.clear(i);
    p.free += 1;
    if i < p.hint {
        p.hint = i;
    }
}

pub fn free_contiguous(addr: u64, count: usize) {
    for k in 0..count as u64 {
        free_frame(addr + k * FRAME_SIZE);
    }
}

/// (allocatable frames, free frames)
pub fn stats() -> (usize, usize) {
    let p = PMM.lock();
    (p.total, p.free)
}

pub fn max_phys() -> u64 {
    PMM.lock().max_phys
}

pub fn regions() -> ([Region; MAX_REGIONS], usize) {
    let p = PMM.lock();
    (p.regions, p.nregions)
}
#![allow(dead_code)]
use super::security::POLICY;
use core::arch::{
    asm,
    x86_64::{__cpuid, __cpuid_count},
};
use core::sync::atomic::{AtomicBool, Ordering};

static NX_ENABLED: AtomicBool = AtomicBool::new(false);
static SMAP_ENABLED: AtomicBool = AtomicBool::new(false);

pub fn nx_enabled() -> bool {
    NX_ENABLED.load(Ordering::Relaxed)
}

#[derive(Clone, Copy, Debug, Default)]
pub struct CpuFeatures {
    pub nx: bool,
    pub smep: bool,
    pub smap: bool,
    pub rdrand: bool,
}

const EFER: u32 = 0xC000_0080;

fn rdmsr(msr: u32) -> u64 {
    let (lo, hi): (u32, u32);
    unsafe { asm!("rdmsr", in("ecx") msr, out("eax") lo, out("edx") hi, options(nomem, nostack)) };
    ((hi as u64) << 32) | lo as u64
}
unsafe fn wrmsr(msr: u32, v: u64) {
    unsafe {
        asm!("wrmsr", in("ecx") msr, in("eax") v as u32, in("edx") (v >> 32) as u32,
         options(nomem, nostack))
    };
}

pub fn detect() -> CpuFeatures {
    let max_basic = __cpuid(0).eax;
    let max_ext = __cpuid(0x8000_0000).eax;
    let nx = max_ext >= 0x8000_0001 && (__cpuid(0x8000_0001).edx >> 20) & 1 == 1;
    let (smep, smap) = if max_basic >= 7 {
        let l7 = __cpuid_count(7, 0);
        ((l7.ebx >> 7) & 1 == 1, (l7.ebx >> 20) & 1 == 1)
    } else {
        (false, false)
    };
    let rdrand = (__cpuid(1).ecx >> 30) & 1 == 1;
    CpuFeatures {
        nx,
        smep,
        smap,
        rdrand,
    }
}

pub fn init() -> CpuFeatures {
    let f = detect();
    if POLICY.enforce_nx && !f.nx {
        panic!("CPU lacks NX but policy requires it");
    }
    unsafe {
        if f.nx {
            wrmsr(EFER, rdmsr(EFER) | (1 << 11)); // EFER.NXE
            NX_ENABLED.store(true, Ordering::Relaxed);
        }
        let mut cr0: u64;
        asm!("mov {}, cr0", out(reg) cr0, options(nomem, nostack));
        cr0 |= 1 << 16; // CR0.WP: supervisor honors read-only pages
        asm!("mov cr0, {}", in(reg) cr0, options(nomem, nostack));

        let mut cr4: u64;
        asm!("mov {}, cr4", out(reg) cr4, options(nomem, nostack));
        if f.smep {
            cr4 |= 1 << 21;
            SMAP_ENABLED.store(true, Ordering::Relaxed);
        }
        if f.smap {
            cr4 |= 1 << 21;
            SMAP_ENABLED.store(true, Ordering::Relaxed);
        } // user copies will need stac/clac later
        asm!("mov cr4, {}", in(reg) cr4, options(nomem, nostack));
    }
    f
}

/// Runs `f` with supervisor access to user pages allowed (stac/clac when SMAP is on).
/// Use it for copy_from_user / copy_to_user style code.
pub fn user_access<T>(f: impl FnOnce() -> T) -> T {
    let smap = SMAP_ENABLED.load(Ordering::Relaxed);
    if smap {
        unsafe { asm!("stac", options(nomem, nostack)) };
    }
    let r = f();
    if smap {
        unsafe { asm!("clac", options(nomem, nostack)) };
    }
    r
}

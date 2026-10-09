use super::Package;
use crate::kprintln;
use core::arch::x86_64::__cpuid;
use core::sync::atomic::AtomicBool;

pub static PACKAGE: Package = Package {
    name: "sysinfo",
    version: "0.1.0",
    description: "Shows basic system information",
    builtin: true,
    installed: AtomicBool::new(true),
    entry: main,
};

fn main(_args: &[&str]) {
    #[allow(unused_unsafe)]
    let r = unsafe { __cpuid(0) };

    let mut vendor = [0u8; 12];
    vendor[0..4].copy_from_slice(&r.ebx.to_le_bytes());
    vendor[4..8].copy_from_slice(&r.edx.to_le_bytes());
    vendor[8..12].copy_from_slice(&r.ecx.to_le_bytes());

    kprintln!("OS:      ZodiacOS 0.1.0");
    kprintln!("Arch:    x86_64 (long mode)");
    kprintln!("CPU:     {}", core::str::from_utf8(&vendor).unwrap_or("unknown"));
    kprintln!("Display: VGA text 80x25");
}
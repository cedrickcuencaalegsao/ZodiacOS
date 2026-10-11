// kernel/src/drivers/mod.rs
#[cfg(target_arch = "x86_64")]
pub mod keyboard; // PS/2
#[cfg(target_arch = "x86_64")]
pub mod rtc; // CMOS clock
pub mod serial; // 16550 on x86, PL011 on AArch64 later
pub mod storage;
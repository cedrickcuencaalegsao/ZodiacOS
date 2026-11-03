#![allow(dead_code)]
use super::{protection, virtual_memory as vm};

pub struct MemoryPolicy {
    pub enforce_wx: bool,
    pub enforce_nx: bool,
    pub zero_on_free: bool,
    pub guard_pages: bool,
}

pub const POLICY: MemoryPolicy = MemoryPolicy {
    enforce_wx: true,
    enforce_nx: true,
    zero_on_free: true,
    guard_pages: true,
};

#[derive(Debug)]
pub enum MapError { WxViolation, Misaligned, OutOfFrames, AlreadyMapped, NotMapped, OutOfRange }

#[derive(Clone, Copy, Debug)]
pub struct Perm { pub write: bool, pub exec: bool, pub user: bool, pub uncached: bool }

impl Perm {
    pub const KERNEL_RX:   Perm = Perm { write: false, exec: true,  user: false, uncached: false };
    pub const KERNEL_RO:   Perm = Perm { write: false, exec: false, user: false, uncached: false };
    pub const KERNEL_RW:   Perm = Perm { write: true,  exec: false, user: false, uncached: false };
    pub const KERNEL_MMIO: Perm = Perm { write: true,  exec: false, user: false, uncached: true  };
    pub const USER_RO:     Perm = Perm { write: false, exec: false, user: true,  uncached: false };
    pub const USER_RW:     Perm = Perm { write: true,  exec: false, user: true,  uncached: false };
    pub const USER_RX:     Perm = Perm { write: false, exec: true,  user: true,  uncached: false };

    /// The single choke point where W^X is enforced.
    pub fn leaf_flags(self) -> Result<u64, MapError> {
        if POLICY.enforce_wx && self.write && self.exec {
            return Err(MapError::WxViolation);
        }
        let mut f = vm::PRESENT;
        if self.write    { f |= vm::WRITABLE; }
        if self.user     { f |= vm::USER; }
        if self.uncached { f |= vm::PWT | vm::PCD; }
        if !self.exec && protection::nx_enabled() { f |= vm::NX; }
        Ok(f)
    }
}
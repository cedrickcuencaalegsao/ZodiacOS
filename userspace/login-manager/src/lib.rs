#![no_std]
extern crate alloc;

use alloc::string::String;

pub mod authentication;
pub mod greeter;
pub mod session;
pub mod user_accounts;

/// Everything the login manager needs from the outside world.
/// The kernel implements this today; a syscall wrapper will later.
pub trait Platform {
    fn write_str(&mut self, s: &str);
    fn clear(&mut self);
    /// Blocking line read. When `echo` is false, show nothing (passwords).
    fn read_line(&mut self, echo: bool) -> String;
    fn random_bytes(&mut self, buf: &mut [u8]);
    fn sleep_ms(&mut self, ms: u64);
}
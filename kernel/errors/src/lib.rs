#![no_std]
pub mod error_enum;
pub mod fmt_display_error;
pub mod impl_error;

pub use error_enum::Error;

pub type Result<T> = core::result::Result<T, Error>;

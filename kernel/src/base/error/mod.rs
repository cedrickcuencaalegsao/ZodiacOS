pub mod impl_error;
pub mod fmt_display_error;
pub mod error_enum;

pub use error_enum::Error;

pub type Result<T> = core::result::Result<T, Error>;
// kernel/src/memory/virtual_memory.rs
//! Architecture-neutral view of paging. For now it re-exports the x86_64 page
//! table code; the AArch64 version will expose the same names.
pub use crate::arch::current::paging::*;
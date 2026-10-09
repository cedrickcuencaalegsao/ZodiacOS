use core::sync::atomic::AtomicBool;

// Register new packages here: 1) add a `pub mod`, 2) add it to ALL.
pub mod hello;
pub mod sysinfo;

pub struct Package {
    pub name: &'static str,
    pub version: &'static str,
    pub description: &'static str,
    pub builtin: bool,
    pub installed: AtomicBool,
    pub entry: fn(&[&str]),
}

pub static ALL: &[&Package] = &[&hello::PACKAGE, &sysinfo::PACKAGE];

pub fn find(name: &str) -> Option<&'static Package> {
    ALL.iter().copied().find(|p| p.name == name)
}
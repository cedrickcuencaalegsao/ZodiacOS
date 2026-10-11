use super::Package;
use crate::kprintln;
use core::sync::atomic::AtomicBool;

pub static PACKAGE: Package = Package {
    name: "hello",
    version: "0.1.0",
    description: "Prints a greeting",
    builtin: false,
    installed: AtomicBool::new(false),
    entry: main,
};

fn main(args: &[&str]) {
    match args.first() {
        Some(who) => kprintln!("Hello, {}! Welcome to ZodiacOS.", who),
        None => kprintln!("Hello from a ZodiacOS package!"),
    }
}
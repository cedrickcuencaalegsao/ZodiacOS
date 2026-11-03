use crate::memory::{
    allocator, heap, layout, physical, protection,
    security::Perm,
    virtual_memory::{self as vm, AddressSpace, Mapping, USER_BASE},
};
use crate::{kprint, kprintln};
use alloc::{boxed::Box, format, string::String, vec::Vec};

pub fn run(cmd: &str, args: &[&str]) {
    match cmd {
        "meminfo" => meminfo(),
        "memmap" => memmap(),
        "vmem" => vmem(args),
        "memtest" => memtest(),
        "vmtest" => vmtest(),
        _ => {}
    }
}

fn mib(frames: usize) -> usize {
    frames * 4096 / (1024 * 1024)
}

fn meminfo() {
    let (total, free) = physical::stats();
    kprintln!(
        "Physical memory  {} MiB usable, {} MiB in use, {} MiB free ({} frames)",
        mib(total),
        mib(total - free),
        mib(free),
        free
    );
    kprintln!(
        "Kernel heap      {} KiB total, {} bytes in use",
        heap::HEAP_SIZE / 1024,
        allocator::used()
    );
    kprintln!("Page tables      CR3 = {:#x}", vm::current());
}

fn memmap() {
    let (regions, count) = physical::regions();
    kprintln!("{:<20} {:<20} {}", "START", "END", "TYPE");
    for r in &regions[..count] {
        kprintln!("{:#018x}   {:#018x}   {}", r.base, r.base + r.len, r.kind_name());
    }
}

fn perms(m: &Mapping) -> &'static str {
    match (m.writable, !m.nx) {
        (true, true) => "rwx",
        (true, false) => "rw-",
        (false, true) => "r-x",
        (false, false) => "r--",
    }
}

fn describe(pml4: u64, virt: u64) {
    match vm::translate(pml4, virt) {
        Some(m) => kprintln!(
            "{:#x} -> {:#x}  {}  {}  ({} KiB page)",
            virt,
            m.phys,
            perms(&m),
            if m.user { "user" } else { "kernel only" },
            m.size / 1024
        ),
        None => kprintln!("{:#x} is not mapped", virt),
    }
}

fn vmem(args: &[&str]) {
    let addr = args
        .first()
        .and_then(|a| u64::from_str_radix(a.trim_start_matches("0x"), 16).ok());
    match addr {
        Some(a) => describe(vm::current(), a),
        None => kprintln!("usage: vmem <hex address>   e.g. vmem 0xb8000"),
    }
}

fn memtest() {
    let before = allocator::used();

    let mut squares: Vec<u64> = Vec::new();
    for i in 0..1000u64 {
        squares.push(i * i);
    }
    let sum: u64 = squares.iter().sum();
    let text: String = format!("1000 squares add up to {}", sum);
    let block = Box::new([7u8; 4096]);

    kprintln!("{}", text);
    kprintln!(
        "heap in use: {} bytes (was {}), 4 KiB block at {:p}",
        allocator::used(),
        before,
        block.as_ref()
    );

    drop(squares);
    drop(text);
    drop(block);
    kprintln!("after freeing: {} bytes", allocator::used());
}

/// Builds a second address space, maps a page into it, reads it back, destroys it.
fn vmtest() {
    let mut space = match AddressSpace::new_user() {
        Ok(s) => s,
        Err(e) => {
            kprintln!("vmtest: cannot create address space: {:?}", e);
            return;
        }
    };
    let Some(frame) = physical::alloc_frame() else {
        kprintln!("vmtest: out of memory");
        return; // `space` is dropped here and frees its tables
    };
    if let Err(e) = space.map_4k(USER_BASE, frame, Perm::USER_RW) {
        kprintln!("vmtest: map failed: {:?}", e);
        physical::free_frame(frame); // not mapped, so we still own it
        return;
    }
    // From here on the frame belongs to `space` and is freed when it is dropped.
    unsafe { (vm::phys_to_virt(frame) as *mut u64).write(0xC0FFEE) };

    let kernel = vm::current();
    kprint!("user page (new space):   ");
    describe(space.root(), USER_BASE);
    kprint!("user page (kernel view): ");
    describe(kernel, USER_BASE);
    kprint!("kernel code (new space): ");
    describe(space.root(), layout::text_start());

    unsafe { space.activate() };
    // SMAP makes the kernel fault on user pages unless access is opened explicitly.
    let value = protection::user_access(|| unsafe {
        core::ptr::read_volatile(USER_BASE as *const u64)
    });
    unsafe { vm::load_cr3(kernel) };
    kprintln!("read through the new address space: {:#x} (expected 0xc0ffee)", value);

    drop(space);
    kprintln!("address space destroyed");
}
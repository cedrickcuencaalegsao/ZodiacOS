use crate::paging::{self, Mapping};
use crate::{heap, pmm};
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
    let (total, free) = pmm::stats();
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
        heap::used()
    );
    kprintln!("Page tables      CR3 = {:#x}", paging::current());
}

fn memmap() {
    let (regions, count) = pmm::regions();
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
    match paging::translate(pml4, virt) {
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
        Some(a) => describe(paging::current(), a),
        None => kprintln!("usage: vmem <hex address>   e.g. vmem 0xb8000"),
    }
}

fn memtest() {
    let before = heap::used();

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
        heap::used(),
        before,
        block.as_ref()
    );

    drop(squares);
    drop(text);
    drop(block);
    kprintln!("after freeing: {} bytes", heap::used());
}

/// Builds a second address space, maps a page into it, reads it back, destroys it.
fn vmtest() {
    let Some(space) = paging::new_address_space() else {
        kprintln!("vmtest: out of memory");
        return;
    };
    let Some(frame) = pmm::alloc_frame() else {
        kprintln!("vmtest: out of memory");
        paging::destroy_address_space(space);
        return;
    };
    let flags = paging::USER | paging::WRITABLE | paging::NX;
    if let Err(e) = paging::map_page(space, paging::USER_BASE, frame, flags) {
        kprintln!("vmtest: map failed: {}", e);
        pmm::free_frame(frame);
        paging::destroy_address_space(space);
        return;
    }
    unsafe { (frame as *mut u64).write(0xC0FFEE) };

    kprint!("user page:   ");
    describe(space, paging::USER_BASE);
    kprint!("kernel code: ");
    describe(space, 0x10000);

    let old = paging::current();
    paging::activate(space);
    let value = unsafe { core::ptr::read_volatile(paging::USER_BASE as *const u64) };
    paging::activate(old);
    kprintln!("read through the new address space: {:#x} (expected 0xc0ffee)", value);

    paging::destroy_address_space(space);
    kprintln!("address space destroyed");
}
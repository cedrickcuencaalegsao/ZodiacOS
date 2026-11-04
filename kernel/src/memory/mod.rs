// kernel/src/memory/mod.rs
pub mod allocator; // the #[global_allocator] (free-list heap)
pub mod heap; // maps the heap region and hands it to the allocator
pub mod layout; // linker-script symbols (kernel image and section addresses)
pub mod memsecurity;
pub mod physical; // frame allocator (E820 map + bitmap)
pub mod protection; // CPUID detection, NXE, CR0.WP, SMEP/SMAP
pub mod security; // Perm, MemoryPolicy, W^X enforcement
pub mod virtual_memory; // page tables, address spaces, kernel remap

use errors::Error;
use shared::kassert;

pub fn init() -> Result<(), Error> {
    protection::init(); // NXE + WP + SMEP/SMAP (before any NX mapping)
    physical::init(); // reads the BIOS memory map written by boot.asm
    virtual_memory::init(physical::max_phys()); // new page tables, per-section perms, load CR3
    heap::init(); // map the heap and start the global allocator

    let (total, free) = physical::stats();
    if total == 0 || free == 0 {
        return Err(Error::OutOfMemory);
    }
    crate::log_info!("memory: {} frames allocatable, {} free", total, free);
    Ok(())
}

/// Quick sanity checks. Call once after `init()` while bringing this up.
pub fn selftest() {
    // W^X must be rejected
    let bad = security::Perm {
        write: true,
        exec: true,
        user: false,
        uncached: false,
    };
    kassert!(
        bad.leaf_flags().is_err(),
        "W^X check did not reject a W+X mapping"
    );

    // frame allocator
    let a = physical::alloc_frame().unwrap();
    let b = physical::alloc_frame().unwrap();
    kassert!(a != b, "frame allocator returned the same frame twice");
    physical::free_frame(a);
    physical::free_frame(b);

    // heap
    let v = alloc::vec![1u64, 2, 3];
    kassert!(
        v.iter().sum::<u64>() == 6,
        "heap allocation returned bad data"
    );

    crate::log_info!("memory: selftest passed");
}

/// Should trigger a page fault with error code 0x3 (present + write).
/// Only call this deliberately, to confirm that .text is really read-only.
#[allow(dead_code)]
pub unsafe fn fault_write_to_text() {
    unsafe {
        (layout::text_start() as *mut u8).write_volatile(0);
    }
}

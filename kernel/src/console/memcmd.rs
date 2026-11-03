use crate::kprintln;
use crate::memory::{
    allocator, heap, layout, physical, protection,
    security::{MapError, POLICY, Perm},
    virtual_memory::{self as vm, AddressSpace, Mapping, USER_BASE, USER_LIMIT},
};
use alloc::{boxed::Box, format, string::String, vec::Vec};
use core::arch::asm;

const FRAME: u64 = 4096;

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

// ---------------------------------------------------------------------------
// Formatting helpers
// ---------------------------------------------------------------------------

fn size_str(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * KIB;
    const GIB: u64 = 1024 * MIB;
    if bytes >= GIB {
        format!("{}.{:02} GiB", bytes / GIB, bytes % GIB * 100 / GIB)
    } else if bytes >= MIB {
        format!("{}.{:02} MiB", bytes / MIB, bytes % MIB * 100 / MIB)
    } else if bytes >= KIB {
        format!("{}.{:02} KiB", bytes / KIB, bytes % KIB * 100 / KIB)
    } else {
        format!("{} bytes", bytes)
    }
}

fn pct(part: u64, whole: u64) -> u64 {
    if whole == 0 { 0 } else { part * 100 / whole }
}

/// A 20-character usage bar, e.g. [#####---------------]
fn bar(part: u64, whole: u64) -> String {
    let filled = if whole == 0 {
        0
    } else {
        (part * 20 / whole).min(20) as usize
    };
    let mut s = String::from("[");
    for i in 0..20 {
        s.push(if i < filled { '#' } else { '-' });
    }
    s.push(']');
    s
}

fn heading(title: &str) {
    kprintln!("");
    kprintln!("== {} ==", title);
}

fn row(label: &str, value: &str) {
    kprintln!("  {:<22} {}", label, value);
}

fn note(text: &str) {
    kprintln!("  note: {}", text);
}

fn yes_no(b: bool) -> &'static str {
    if b { "yes" } else { "no" }
}

fn status(supported: bool, on: bool) -> &'static str {
    if on {
        "enabled"
    } else if supported {
        "supported by the CPU, NOT enabled"
    } else {
        "not supported by this CPU"
    }
}

fn in_heap(addr: usize) -> bool {
    let start = heap::HEAP_START as usize;
    addr >= start && addr < start + heap::HEAP_SIZE
}

// ---------------------------------------------------------------------------
// CPU control registers (read-only, used to show what protection is really on)
// ---------------------------------------------------------------------------

fn read_cr0() -> u64 {
    let v: u64;
    unsafe { asm!("mov {}, cr0", out(reg) v, options(nomem, nostack, preserves_flags)) };
    v
}

fn read_cr4() -> u64 {
    let v: u64;
    unsafe { asm!("mov {}, cr4", out(reg) v, options(nomem, nostack, preserves_flags)) };
    v
}

fn read_efer() -> u64 {
    let (lo, hi): (u32, u32);
    unsafe {
        asm!("rdmsr", in("ecx") 0xC000_0080u32, out("eax") lo, out("edx") hi,
             options(nomem, nostack, preserves_flags));
    }
    ((hi as u64) << 32) | lo as u64
}

// ---------------------------------------------------------------------------
// meminfo
// ---------------------------------------------------------------------------

fn meminfo() {
    let (total, free) = physical::stats();
    let used = total - free;
    let (regions, count) = physical::regions();
    let usable: u64 = regions[..count]
        .iter()
        .filter(|r| r.kind == 1)
        .map(|r| r.len)
        .sum();
    let allocatable = total as u64 * FRAME;
    let reserved = usable.saturating_sub(allocatable);

    heading("Physical memory (RAM)");
    row(
        "Highest usable address",
        &format!(
            "{:#x} ({})",
            physical::max_phys(),
            size_str(physical::max_phys())
        ),
    );
    row("Usable RAM", &size_str(usable));
    row(
        "Reserved at boot",
        &format!(
            "{}  (BIOS area, kernel image, frame bitmap)",
            size_str(reserved)
        ),
    );
    row(
        "Allocatable",
        &format!("{}  ({} frames of 4 KiB)", size_str(allocatable), total),
    );
    row(
        "In use",
        &format!(
            "{}  ({} frames)  {} {}%",
            size_str(used as u64 * FRAME),
            used,
            bar(used as u64, total as u64),
            pct(used as u64, total as u64)
        ),
    );
    row(
        "Free",
        &format!("{}  ({} frames)", size_str(free as u64 * FRAME), free),
    );
    note(
        "'In use' counts frames handed out since boot: page tables, kernel stacks, the heap, and user pages.",
    );

    heading("Kernel image (loaded in RAM)");
    let ks = layout::kernel_start();
    let ke = layout::kernel_end();
    row(
        "Range",
        &format!("{:#x} - {:#x}  ({})", ks, ke, size_str(ke - ks)),
    );
    row(
        ".text  (code)",
        &format!(
            "{:#x} - {:#x}  {}  read + execute",
            layout::text_start(),
            layout::text_end(),
            size_str(layout::text_end() - layout::text_start())
        ),
    );
    row(
        ".rodata (constants)",
        &format!(
            "{:#x} - {:#x}  {}  read only",
            layout::rodata_start(),
            layout::rodata_end(),
            size_str(layout::rodata_end() - layout::rodata_start())
        ),
    );
    row(
        ".data/.bss (variables)",
        &format!(
            "{:#x} - {:#x}  {}  read + write, no execute",
            layout::data_start(),
            ke,
            size_str(ke - layout::data_start())
        ),
    );

    heading("Kernel heap (Box, Vec, String ...)");
    let hused = allocator::used() as u64;
    let hsize = allocator::size() as u64;
    row(
        "Virtual range",
        &format!(
            "{:#x} - {:#x}",
            heap::HEAP_START,
            heap::HEAP_START + heap::HEAP_SIZE as u64
        ),
    );
    row("Size", &size_str(hsize));
    row(
        "In use",
        &format!(
            "{}  {} {}%",
            size_str(hused),
            bar(hused, hsize),
            pct(hused, hsize)
        ),
    );
    row(
        "Free",
        &format!(
            "{}  (may be split into pieces, so one big allocation can still fail)",
            size_str(hsize - hused)
        ),
    );
    note("The heap sits in its own address range with unmapped guard pages on both sides.");

    heading("Paging (virtual memory)");
    row("Active page tables", &format!("CR3 = {:#x}", vm::current()));
    row("Page size", "4 KiB (2 MiB pages for the identity map)");
    row(
        "Identity map",
        "first 4 GiB, virtual address == physical address",
    );
    row(
        "User space",
        &format!(
            "{:#x} - {:#x}  (private per process)",
            USER_BASE, USER_LIMIT
        ),
    );

    heading("Memory protection");
    let cpu = protection::detect();
    let efer = read_efer();
    let cr0 = read_cr0();
    let cr4 = read_cr4();
    row("NX (no-execute)", status(cpu.nx, efer & (1 << 11) != 0));
    row(
        "Write protect (WP)",
        if cr0 & (1 << 16) != 0 {
            "enabled: the kernel also obeys read-only pages"
        } else {
            "NOT enabled"
        },
    );
    row("SMEP", status(cpu.smep, cr4 & (1 << 20) != 0));
    row("SMAP", status(cpu.smap, cr4 & (1 << 21) != 0));
    row(
        "W^X policy",
        if POLICY.enforce_wx {
            "enforced: no page can be writable and executable"
        } else {
            "off"
        },
    );
    row("Zero on free", yes_no(POLICY.zero_on_free));
    note(
        "SMEP blocks the kernel from running user code; SMAP blocks it from touching user memory by accident.",
    );
    if !cpu.smep || !cpu.smap {
        note("Start QEMU with '-cpu max' to get SMEP and SMAP.");
    }
}

// ---------------------------------------------------------------------------
// memmap
// ---------------------------------------------------------------------------

fn memmap() {
    let (regions, count) = physical::regions();

    heading("Physical memory map (from the BIOS, INT 0x15 E820)");
    kprintln!(
        "  {:<4} {:<20} {:<20} {:<12} {}",
        "#",
        "START",
        "END",
        "SIZE",
        "TYPE"
    );
    for (i, r) in regions[..count].iter().enumerate() {
        kprintln!(
            "  {:<4} {:#018x}   {:#018x}   {:<12} {}",
            i,
            r.base,
            r.base + r.len,
            size_str(r.len),
            r.kind_name()
        );
    }

    heading("Totals by type");
    for kind in 1..=5u32 {
        let mut n = 0;
        let mut bytes = 0u64;
        let mut name = "";
        for r in regions[..count].iter().filter(|r| r.kind == kind) {
            n += 1;
            bytes += r.len;
            name = r.kind_name();
        }
        if n > 0 {
            row(name, &format!("{}  in {} region(s)", size_str(bytes), n));
        }
    }

    heading("How to read this");
    note("usable: ordinary RAM the kernel hands out as frames.");
    note("reserved: firmware, device memory and the BIOS area. Never used.");
    note("ACPI reclaimable: can be reused after the ACPI tables are read.");
    note("ACPI NVS / bad memory: never touched.");
    kprintln!("");
    row(
        "Kernel image",
        &format!(
            "{:#x} - {:#x}  ({})",
            layout::kernel_start(),
            layout::kernel_end(),
            size_str(layout::kernel_end() - layout::kernel_start())
        ),
    );
    note(
        "Everything below the end of the kernel and its frame bitmap is kept out of the allocator.",
    );
}

// ---------------------------------------------------------------------------
// vmem
// ---------------------------------------------------------------------------

fn parse_hex(s: &str) -> Option<u64> {
    u64::from_str_radix(s.trim_start_matches("0x").trim_start_matches("0X"), 16).ok()
}

fn perms(m: &Mapping) -> &'static str {
    match (m.writable, !m.nx) {
        (true, true) => "rwx",
        (true, false) => "rw-",
        (false, true) => "r-x",
        (false, false) => "r--",
    }
}

/// Which part of the virtual address space an address belongs to.
fn region_name(addr: u64) -> &'static str {
    if (layout::text_start()..layout::text_end()).contains(&addr) {
        "kernel code (.text)"
    } else if (layout::rodata_start()..layout::rodata_end()).contains(&addr) {
        "kernel constants (.rodata)"
    } else if (layout::data_start()..layout::kernel_end()).contains(&addr) {
        "kernel data (.data / .bss)"
    } else if (heap::HEAP_START..heap::HEAP_START + heap::HEAP_SIZE as u64).contains(&addr) {
        "kernel heap"
    } else if (vm::STACK_REGION..vm::STACK_REGION + (1 << 30)).contains(&addr) {
        "kernel stacks (each has an unmapped guard page below it)"
    } else if (vm::MMIO_BASE..vm::MMIO_BASE + (1 << 30)).contains(&addr) {
        "device memory window (MMIO)"
    } else if (USER_BASE..USER_LIMIT).contains(&addr) {
        "user space (private to each process)"
    } else if addr < (4u64 << 30) {
        if addr >= physical::max_phys() {
            "device memory (identity map, uncached)"
        } else {
            "RAM (identity map: virtual == physical)"
        }
    } else {
        "not part of any region the kernel uses"
    }
}

fn vmem(args: &[&str]) {
    let Some(addr) = args.first().and_then(|a| parse_hex(a)) else {
        kprintln!("usage: vmem <hex address>");
        kprintln!("  Shows how the CPU translates a virtual address: which region it is in,");
        kprintln!("  which page-table entries it goes through, the physical address behind it,");
        kprintln!("  and who may read, write or execute it.");
        kprintln!("examples:");
        kprintln!("  vmem 0xb8000      the VGA text buffer");
        kprintln!("  vmem 0x100000     the start of the kernel image (code)");
        return;
    };

    heading(&format!("Virtual address {:#x}", addr));

    // x86_64 only uses 48 bits; bits 63..48 must copy bit 47.
    let top = addr >> 47;
    if top != 0 && top != 0x1_FFFF {
        row(
            "Canonical",
            "no: the CPU would raise a general protection fault (#GP) on any access",
        );
        return;
    }

    row("Region", region_name(addr));
    row(
        "Page-table path",
        &format!(
            "PML4[{}] -> PDPT[{}] -> PD[{}] -> PT[{}] + offset {:#x}",
            (addr >> 39) & 0x1ff,
            (addr >> 30) & 0x1ff,
            (addr >> 21) & 0x1ff,
            (addr >> 12) & 0x1ff,
            addr & 0xfff
        ),
    );

    match vm::translate(vm::current(), addr) {
        Some(m) => {
            row("Mapped", "yes");
            row("Physical address", &format!("{:#x}", m.phys));
            row(
                "Page size",
                &format!(
                    "{}{}",
                    size_str(m.size),
                    if m.size > FRAME {
                        "  (huge page: the PT level is skipped)"
                    } else {
                        ""
                    }
                ),
            );
            row(
                "Permissions",
                &format!(
                    "{}  (read: yes, write: {}, execute: {})",
                    perms(&m),
                    yes_no(m.writable),
                    yes_no(!m.nx)
                ),
            );
            row(
                "Accessible from",
                if m.user {
                    "user mode and kernel mode"
                } else {
                    "kernel mode only (a user program would fault)"
                },
            );
            if m.writable && !m.nx {
                note("writable AND executable: this breaks W^X and should never happen.");
            }
            if !m.writable && !m.nx {
                note("code page: it can run but not be modified.");
            }
        }
        None => {
            row("Mapped", "no");
            note(
                "Any access to this address would raise a page fault (#PF) with CR2 = this address.",
            );
            note("The kernel uses this for guard pages and for memory that was never allocated.");
        }
    }
}

// ---------------------------------------------------------------------------
// memtest
// ---------------------------------------------------------------------------

#[repr(align(64))]
struct Aligned([u8; 64]);

fn memtest() {
    let mut passed = 0u32;
    let mut total = 0u32;
    let mut check = |name: &str, ok: bool| {
        total += 1;
        if ok {
            passed += 1;
        }
        kprintln!("    [{}] {}", if ok { "PASS" } else { "FAIL" }, name);
    };

    heading("Kernel heap test");
    let baseline = allocator::used();
    kprintln!(
        "  Heap before the test: {} in use of {}",
        size_str(baseline as u64),
        size_str(allocator::size() as u64)
    );

    kprintln!("");
    kprintln!("  1. Vec<u64>: push 1000 numbers (the buffer grows and moves several times)");
    let mut squares: Vec<u64> = Vec::new();
    for i in 0..1000u64 {
        squares.push(i * i);
    }
    let sum: u64 = squares.iter().sum();
    kprintln!(
        "     buffer at {:p}, capacity {} ({}), heap now {}",
        squares.as_ptr(),
        squares.capacity(),
        size_str(squares.capacity() as u64 * 8),
        size_str(allocator::used() as u64)
    );
    check(
        "buffer is inside the heap range",
        in_heap(squares.as_ptr() as usize),
    );
    check(
        "buffer is 16-byte aligned",
        squares.as_ptr() as usize % 16 == 0,
    );
    check("sum of squares 0..999 is 332833500", sum == 332_833_500);

    kprintln!("");
    kprintln!("  2. String: format a text");
    let text: String = format!("1000 squares add up to {}", sum);
    kprintln!(
        "     \"{}\" ({} bytes) at {:p}",
        text,
        text.len(),
        text.as_ptr()
    );
    check(
        "string is inside the heap range",
        in_heap(text.as_ptr() as usize),
    );

    kprintln!("");
    kprintln!("  3. Box<[u8; 4096]>: one big block filled with 7");
    let block = Box::new([7u8; 4096]);
    kprintln!("     block at {:p}", block.as_ref());
    check(
        "block is inside the heap range",
        in_heap(block.as_ptr() as usize),
    );
    check(
        "all 4096 bytes read back as 7",
        block.iter().all(|&b| b == 7),
    );

    kprintln!("");
    kprintln!("  4. Box with 64-byte alignment");
    let aligned = Box::new(Aligned([1u8; 64]));
    let aligned_addr = &*aligned as *const Aligned as usize;
    kprintln!(
        "     block at {:#x} (first byte = {})",
        aligned_addr,
        aligned.0[0]
    );
    check("address is a multiple of 64", aligned_addr % 64 == 0);

    kprintln!("");
    kprintln!(
        "  5. Free-list reuse: allocate three 1 KiB blocks, free the middle one, allocate again"
    );
    let b1 = Box::new([1u8; 1024]);
    let b2 = Box::new([2u8; 1024]);
    let b3 = Box::new([3u8; 1024]);
    let hole = b2.as_ptr() as usize;
    drop(b2);
    let b4 = Box::new([4u8; 1024]);
    let reused = b4.as_ptr() as usize == hole;
    kprintln!(
        "     freed block was at {:#x}, new block at {:#x}: {}",
        hole,
        b4.as_ptr() as usize,
        if reused {
            "the hole was reused"
        } else {
            "a different (lower) hole was used instead"
        }
    );
    check(
        "blocks 1 and 3 kept their contents",
        b1[0] == 1 && b3[1023] == 3,
    );
    drop(b1);
    drop(b3);
    drop(b4);

    kprintln!("");
    kprintln!("  6. Free everything and check for leaks");
    let peak = allocator::used();
    drop(squares);
    drop(text);
    drop(block);
    drop(aligned);
    let after = allocator::used();
    kprintln!(
        "     heap use: {} before, {} at the peak, {} after freeing",
        size_str(baseline as u64),
        size_str(peak as u64),
        size_str(after as u64)
    );
    check(
        "heap returned to its starting size (no leak)",
        after == baseline,
    );

    kprintln!("");
    if passed == total {
        kprintln!("  RESULT: all {} checks passed", total);
    } else {
        kprintln!("  RESULT: {} of {} checks FAILED", total - passed, total);
    }
}

// ---------------------------------------------------------------------------
// vmtest
// ---------------------------------------------------------------------------

fn vmtest() {
    let mut passed = 0u32;
    let mut total = 0u32;
    let mut check = |name: &str, ok: bool| {
        total += 1;
        if ok {
            passed += 1;
        }
        kprintln!("    [{}] {}", if ok { "PASS" } else { "FAIL" }, name);
    };

    heading("Address space test");
    let kernel = vm::current();
    let (_, free_before) = physical::stats();
    kprintln!(
        "  Kernel page tables: CR3 = {:#x}, free frames: {}",
        kernel,
        free_before
    );

    kprintln!("");
    kprintln!("  1. Create a process address space");
    let mut space = match AddressSpace::new_user() {
        Ok(s) => s,
        Err(e) => {
            kprintln!("     cannot create address space: {:?}", e);
            return;
        }
    };
    kprintln!(
        "     new top-level page table (PML4) at {:#x}",
        space.root()
    );
    check(
        "it is a different address space from the kernel's",
        space.root() != kernel,
    );

    kprintln!("");
    kprintln!("  2. Map one writable user page at {:#x}", USER_BASE);
    let Some(frame) = physical::alloc_frame() else {
        kprintln!("     out of memory");
        return; // `space` is dropped here and frees its tables
    };
    if let Err(e) = space.map_4k(USER_BASE, frame, Perm::USER_RW) {
        kprintln!("     map failed: {:?}", e);
        physical::free_frame(frame); // not mapped, so we still own it
        return;
    }
    // From here on the frame belongs to `space` and is freed when it is dropped.
    unsafe { (vm::phys_to_virt(frame) as *mut u64).write(0xC0FFEE) };
    kprintln!(
        "     backed by physical frame {:#x}, which now holds 0xC0FFEE",
        frame
    );

    kprintln!("");
    kprintln!("  3. Look at the page through the new tables");
    let user_view = vm::translate(space.root(), USER_BASE);
    match user_view {
        Some(m) => kprintln!(
            "     {:#x} -> {:#x}  {}  {}",
            USER_BASE,
            m.phys,
            perms(&m),
            if m.user { "user" } else { "kernel only" }
        ),
        None => kprintln!("     {:#x} is not mapped", USER_BASE),
    }
    check(
        "the page is mapped to the frame we gave it",
        matches!(user_view, Some(m) if m.phys == frame),
    );
    check(
        "it is readable and writable by user mode",
        matches!(user_view, Some(m) if m.user && m.writable),
    );
    check(
        "it is not executable (NX)",
        matches!(user_view, Some(m) if m.nx),
    );

    kprintln!("");
    kprintln!("  4. Isolation: does the kernel's own address space see that page?");
    let kernel_view = vm::translate(kernel, USER_BASE);
    match kernel_view {
        Some(_) => kprintln!(
            "     {:#x} is mapped in the kernel tables (it should not be)",
            USER_BASE
        ),
        None => kprintln!("     {:#x} is not mapped in the kernel tables", USER_BASE),
    }
    check(
        "the page exists only in the new address space",
        kernel_view.is_none(),
    );

    kprintln!("");
    kprintln!("  5. Is the kernel itself protected inside the new address space?");
    let code_view = vm::translate(space.root(), layout::text_start());
    match code_view {
        Some(m) => kprintln!(
            "     kernel code {:#x}: {}  {}",
            layout::text_start(),
            perms(&m),
            if m.user { "user" } else { "kernel only" }
        ),
        None => {
            kprintln!("     kernel code is not mapped (the CPU could not handle interrupts here)")
        }
    }
    check(
        "kernel code is mapped (interrupts and syscalls still work)",
        code_view.is_some(),
    );
    check(
        "...but only for kernel mode",
        matches!(code_view, Some(m) if !m.user),
    );
    check(
        "...and it is read-only",
        matches!(code_view, Some(m) if !m.writable),
    );

    kprintln!("");
    kprintln!("  6. The mapper must refuse unsafe requests");
    let wx = Perm {
        write: true,
        exec: true,
        user: true,
        uncached: false,
    };
    let r = space.map_4k(USER_BASE + FRAME, 0, wx);
    kprintln!("     writable + executable page -> {:?}", r);
    check(
        "W^X violation is rejected",
        matches!(r, Err(MapError::WxViolation)),
    );
    let r = space.map_4k(0x1000, 0, Perm::USER_RW);
    kprintln!("     user page at 0x1000 (outside user space) -> {:?}", r);
    check(
        "user page outside the user range is rejected",
        matches!(r, Err(MapError::OutOfRange)),
    );

    kprintln!("");
    kprintln!("  7. Switch to the new address space and use the page");
    // Interrupts stay off while the page tables are swapped: a timer tick could
    // otherwise make the scheduler change CR3 under us.
    let irq = crate::arch::save_and_disable_interrupts();
    unsafe { space.activate() };
    // SMAP makes the kernel fault on user pages unless access is opened explicitly.
    let (first, second) = protection::user_access(|| unsafe {
        let p = USER_BASE as *mut u64;
        let first = p.read_volatile();
        p.write_volatile(0xBEEF);
        (first, p.read_volatile())
    });
    unsafe { vm::load_cr3(kernel) };
    crate::arch::restore_interrupts(irq);
    let via_kernel = unsafe { (vm::phys_to_virt(frame) as *const u64).read_volatile() };
    kprintln!(
        "     read at {:#x}: {:#x}, after writing 0xBEEF: {:#x}",
        USER_BASE,
        first,
        second
    );
    kprintln!(
        "     the same frame {:#x}, read directly by the kernel: {:#x}",
        frame,
        via_kernel
    );
    check(
        "the user address reads back what the kernel wrote to the frame (0xC0FFEE)",
        first == 0xC0FFEE,
    );
    check(
        "a write through the user address lands in the physical frame",
        via_kernel == 0xBEEF,
    );

    kprintln!("");
    kprintln!("  8. Destroy the address space and check for leaks");
    drop(space);
    let (_, free_after) = physical::stats();
    kprintln!(
        "     free frames: {} before, {} after ({} difference)",
        free_before,
        free_after,
        free_before as i64 - free_after as i64
    );
    check(
        "every page table and the user frame were returned",
        free_after == free_before,
    );

    kprintln!("");
    if passed == total {
        kprintln!("  RESULT: all {} checks passed", total);
    } else {
        kprintln!("  RESULT: {} of {} checks FAILED", total - passed, total);
    }
}

# ZodiacOS Memory Security

> **Status:** Proposed architecture and implementation roadmap  
> **Subsystem:** ZodiacOS kernel memory management and protection  
> **Target architecture:** x86_64 initially  
> **Implementation language:** Rust and Assembly

## 1. Purpose

ZodiacOS Memory Security defines how the kernel protects memory belonging to the kernel, processes, threads, applications, devices, and sensitive system data.

The goal is to make memory access explicit, isolated, permission-controlled, and auditable. An application must not be able to read or modify kernel memory, or another application's private memory, merely because it knows an address or process ID.

Rust's ownership and type systems provide valuable protection, but they do not replace hardware-enforced memory isolation. Unsafe Rust, assembly, kernel interfaces, device access, DMA, and bugs in privileged code still require explicit defenses.

## 2. Core security principles

1. **Isolation by default:** Kernel memory and each process's private memory are isolated.
2. **Least privilege:** Every mapping receives only the permissions it needs.
3. **Explicit sharing:** Memory is shared only through an authorized mechanism.
4. **W^X:** A page must not be writable and executable at the same time.
5. **Ownership and lifetime:** Memory allocations and mappings have an identifiable owner and controlled lifetime.
6. **Defense in depth:** Rust safety, kernel validation, page-table permissions, CPU features, and device protections complement one another.
7. **Secure cleanup:** Sensitive memory is cleared before it is released or reused.
8. **Policy backed by enforcement:** A policy setting is meaningful only when the memory manager and hardware actually enforce it.

## 3. Kernel and user memory isolation

The kernel must occupy protected virtual memory that ordinary user applications cannot access.

Use x86_64 virtual memory paging and page-table permission bits to separate:

- Kernel memory
- Process A's private address space
- Process B's private address space
- Explicitly authorized shared-memory regions
- Memory-mapped hardware regions

The page-table User/Supervisor (U/S) bit must mark kernel pages as supervisor-only. User processes must not be able to read or write those pages.

The memory manager should support:

- Separate page-table roots or address spaces for processes.
- Kernel-only mappings.
- User-accessible mappings only where explicitly intended.
- Page-level read, write, and execute permissions.
- Controlled shared-memory mappings.
- Validation of mappings before they are installed.
- Safe address-space changes during process/thread context switches.

The exact kernel mapping strategy (for example, which kernel regions are mapped into each process address space) is an implementation decision, but it must preserve isolation.

## 4. W^X and executable-memory policy

W^X means **Write XOR Execute**: a memory page must not be writable and executable simultaneously.

Suggested permissions:

| Memory region | Read | Write | Execute |
|---|---:|---:|---:|
| Application code | Yes | No | Yes |
| Application data | Yes | Yes | No |
| Application stack | Yes | Yes | No |
| Kernel code | Yes | No | Yes |
| Kernel data | Yes | Yes | No |

Enforce this through page-table permissions and controlled memory-mapping APIs, not merely through a software convention.

Required protections:

- NX (No Execute) page permissions for data pages.
- Write protection for code and other read-only regions.
- Kernel text protection.
- Controlled executable-memory allocation and transitions.
- Validation that no mapping becomes writable and executable at once.

If executable memory must be generated or modified, use a controlled transition: make it writable but non-executable while writing, then make it executable and non-writable before execution.

## 5. Address Space Layout Randomization (ASLR)

ASLR randomizes the locations of important memory regions to make address prediction harder.

Randomization targets include:

- Executable image base addresses.
- Stack locations.
- Heap locations.
- Shared-library mappings.
- Kernel address layout (KASLR).

Use a trustworthy entropy source and avoid predictable fallback values in production. ASLR is defense in depth; it does not replace memory isolation or permission checks.

## 6. Stack protection

Protect both user and kernel stacks against overflow and corruption.

Planned mechanisms:

- Guard pages around stacks.
- Stack overflow detection.
- Stack canaries where appropriate.
- Separate kernel stacks per thread.
- Guarded interrupt and exception stacks.
- Appropriate non-executable stack mappings.

Kernel stacks, interrupt stacks, and exception stacks need particular care because their corruption can compromise the entire system.

## 7. Heap and allocation protection

The heap and allocation subsystem should detect or reduce common memory errors.

Planned mechanisms:

- Allocation-boundary validation.
- Double-free detection.
- Use-after-free mitigation and detection.
- Allocation ownership tracking.
- Memory poisoning in debug builds.
- Safe allocator metadata handling.
- Guard regions or canaries where practical.

Rust ownership and lifetimes help prevent many classes of errors in safe code. They do not eliminate risks in unsafe code, FFI, assembly, allocator internals, or incorrect kernel abstractions.

## 8. Process memory isolation

Every process should have its own virtual address space and private memory.

A process may contain:

- Code and read-only data.
- Writable data and heap.
- One or more thread stacks.
- Memory-mapped resources.
- Explicitly authorized shared regions.

A process must not be able to inspect another process's memory simply by knowing its process ID or a virtual address.

Cross-process communication must use authorized mechanisms, such as:

- Message passing.
- Shared memory with explicit authorization.
- System calls.
- IPC channels.

The kernel must validate the requesting process, the target resource, and the requested permissions before granting access.

## 9. Capability-based memory access and the hybrid model

The selected architectural direction is **C: a hybrid of strict process isolation and capability-based access**.

- Strict isolation is the default for process-private memory.
- Sharing requires an explicit grant.
- Capabilities or protected handles represent authority to access a particular memory region or resource.
- A capability should identify the resource and the permitted operations; possession of a process ID or guessed address is not authority.
- The kernel validates capabilities and permissions when mapping, sharing, or accessing memory.

This model can also be extended to shared memory, devices, and other system resources. It allows legitimate application communication without weakening default isolation.

## 10. Secure memory for sensitive information

Some data needs additional handling because disclosure can compromise users or the system.

Examples include:

- Passwords.
- Encryption keys.
- Authentication tokens.
- Cryptographic session keys.
- Private application data.

Planned protections:

- Dedicated secure-memory allocation APIs.
- Reliable memory zeroization when sensitive data is no longer needed.
- Avoiding sensitive pages in swap or other persistent backing storage.
- Restricting debugger access to sensitive memory.
- Avoiding sensitive information in crash dumps and diagnostic output.
- Hardware-backed key storage where available.

Zeroization must use an implementation that cannot simply be optimized away by the compiler. The API must also define ownership, lifetime, and cleanup behavior.

Sensitive-memory protection is not a guarantee against a fully compromised kernel or every physical attack; the threat model and limitations should be documented.

## 11. Hardware-enforced protections

ZodiacOS should detect CPU capabilities at boot and enable supported protections only when they are available and correctly configured.

| Technology | Purpose |
|---|---|
| NX bit | Prevent execution from pages marked non-executable |
| SMEP | Prevent supervisor-mode instruction execution from user pages |
| SMAP | Restrict supervisor-mode data access to user pages |
| CR0.WP | Enforce read-only page permissions for supervisor writes |
| PCID | Improve address-space switching performance by tagging TLB entries |
| Intel VT-x | Hardware virtualization support for isolated virtual machines |
| IOMMU | Restrict device DMA access to authorized memory |
| CET | Hardware-assisted control-flow protection |

These features are not all interchangeable and do not all provide memory isolation in the same way. PCID is primarily a performance feature; VT-x supports virtualization; IOMMU protects against unauthorized device DMA; CET protects control flow. They complement, rather than replace, page-table permissions and process isolation.

Implementation must include CPU feature detection, safe enablement, correct exception handling, and fallbacks for unsupported hardware. Do not assume every x86_64 processor supports every feature.

## 12. Central memory security policy

Introduce a central policy model so the memory subsystem has a consistent description of the protections ZodiacOS intends to enforce.

Illustrative Rust API:

```rust
pub enum MemoryPermission {
    KernelOnly,
    UserReadOnly,
    UserReadWrite,
    ExecuteOnly,
    ReadExecute,
    Shared,
    Protected,
}

pub struct MemoryPolicy {
    pub enforce_wx: bool,
    pub enforce_nx: bool,
    pub enforce_aslr: bool,
    pub enforce_isolation: bool,
    pub zero_on_free: bool,
    pub guard_pages: bool,
}
```

This is a conceptual starting point, not a complete or sufficient security implementation.

The policy must be translated into validated page-table mappings, allocator behavior, process rules, and hardware configuration. Avoid permission combinations that violate W^X. The kernel should fail safely when a required protection cannot be established, or explicitly report a documented reduced-security mode where appropriate.

## 13. Proposed kernel source organization

The following is a suggested organization for the Rust kernel:

```text
kernel/src/
└── memory/
    ├── mod.rs
    ├── physical/
    │   ├── frame_allocator.rs
    │   └── memory_map.rs
    ├── virtual/
    │   ├── page_table.rs
    │   ├── address_space.rs
    │   └── mapper.rs
    ├── allocator/
    │   ├── heap.rs
    │   └── slab.rs
    ├── security/
    │   ├── permissions.rs
    │   ├── isolation.rs
    │   ├── guard_pages.rs
    │   ├── aslr.rs
    │   ├── zeroize.rs
    │   └── secure_alloc.rs
    └── protection/
        ├── smep.rs
        ├── smap.rs
        ├── nx.rs
        └── cet.rs
```

This layout is a proposal. Files should be introduced as their corresponding features are implemented; the directory structure alone does not provide security.

## 14. Implementation roadmap

### Phase 1 — Memory foundation

- Physical frame allocator.
- Virtual memory manager.
- Page-table management.
- Kernel/user memory separation.

### Phase 2 — Memory protection

- NX protection.
- W^X enforcement.
- Read-only kernel pages.
- Guard pages.
- CR0.WP.

### Phase 3 — Process isolation

- Independent process address spaces.
- Address-space handling during context switches.
- Per-process memory permissions.
- Secure IPC and explicitly authorized shared memory.

### Phase 4 — Advanced protection

- ASLR and KASLR.
- SMEP and SMAP.
- Stack protection.
- Heap corruption detection and mitigation.

### Phase 5 — Secure memory

- Secure allocation.
- Memory zeroization.
- Sensitive-memory protection.
- Crash-dump sanitization.

### Phase 6 — Hardware security

- IOMMU support.
- CET support.
- Hardware-backed key protection.
- Virtualization-based isolation where needed.

## 15. Secure allocation API concepts

Potential interfaces include:

```rust
secure_alloc(size);
secure_free(ptr);
secure_zero(ptr, size);
```

These are illustrative signatures only. A production API should use safe ownership-oriented types rather than expose unrestricted raw pointers where possible. It must define allocation failure, alignment, page permissions, lifetime, cleanup, and whether the allocation is locked against swapping.

`secure_free` must ensure sensitive contents are cleared before the underlying memory is made available for reuse. `secure_zero` must use a compiler-resistant zeroization implementation.

## 16. Security validation and testing

Every implemented protection should have tests that demonstrate enforcement, not merely successful configuration.

Suggested tests include:

- A user process attempting to read kernel-only memory must fault or be denied.
- A process attempting to access another process's private mapping must fault or be denied.
- A write to a read-only page must be rejected.
- Execution from an NX page must fault.
- No page may be both writable and executable.
- Guard-page access must trigger the expected fault.
- Invalid or unauthorized shared-memory requests must be rejected.
- Sensitive allocations must be zeroed before reuse.
- Unsupported CPU features must not be enabled.
- Address-space switching must not expose stale mappings or permissions.
- Device DMA must be constrained when IOMMU support is enabled.

Use emulator testing (including QEMU where appropriate), debug assertions, fault reporting, and hardware testing. Security tests should also cover failure paths and malformed inputs.

## 17. Design decisions and open implementation details

### Agreed direction

- ZodiacOS will use hardware-enforced virtual memory protection.
- Kernel memory is inaccessible to ordinary user applications.
- Processes receive isolated virtual address spaces.
- Memory sharing is explicit and authorized.
- The chosen process-memory model is hybrid: strict isolation plus capability-based access.
- W^X and NX are core protections.
- Stack, heap, and sensitive memory receive additional defenses.
- ASLR/KASLR and supported CPU protections are planned.
- A central memory security policy will describe intended protections.
- Implementation will proceed in the six phases above.

### Details to define during implementation

- Exact virtual address-space layout and kernel mapping strategy.
- Physical and virtual allocator designs.
- Capability representation, lifetime, revocation, and delegation rules.
- IPC and shared-memory APIs.
- Entropy source and randomization strategy.
- Swap policy and sensitive-page pinning behavior.
- Crash-dump and debugger authorization model.
- CPU-feature fallback behavior and minimum supported hardware.
- IOMMU device ownership and DMA mapping policy.
- Secure allocation API types and zeroization guarantees.

## 18. ZodiacOS memory security philosophy

> **No memory access should be trusted by default. Every allocation, mapping, and access must have an explicit owner, permission, and lifetime.**

Memory security is a system-wide responsibility. The allocator, virtual memory manager, process manager, scheduler, system-call interface, IPC subsystem, device drivers, and hardware configuration must work together to enforce this principle.

A secure ZodiacOS does not rely on one language feature or one CPU setting. It combines safe Rust abstractions, carefully reviewed unsafe code and assembly, strict address-space isolation, explicit capabilities, page-level permissions, secure cleanup, and hardware protections.

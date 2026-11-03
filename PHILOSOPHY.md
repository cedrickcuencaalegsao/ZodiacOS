# ZodiacOS

> **Freedom Through Structure. Security Through Design.**

ZodiacOS is an independent operating-system project built from the
ground up. Its goal is to create an open, user-controlled computing
environment where applications and system components have clearly
defined authority, failures are contained, and users retain meaningful
control over their machines.

ZodiacOS takes inspiration from the useful ideas found across
operating-system history---including Unix's simplicity and
composability, Linux's open development culture, Rust's memory-safety
tools, capability-based security, and service isolation---while
developing its own architecture and identity.

> **Project status:** Early development. ZodiacOS currently has a
> bare-metal Rust and Assembly kernel with a command-line shell. The
> principles and future capabilities described below are design goals,
> not claims that every feature is already implemented.

------------------------------------------------------------------------

## Table of Contents

-   [Vision](#vision)
-   [The ZodiacOS Philosophy](#the-zodiacos-philosophy)
-   [The Seven Principles](#the-seven-principles)
-   [Architectural Direction](#architectural-direction)
-   [Security Model](#security-model)
-   [Applications and Compatibility](#applications-and-compatibility)
-   [ZodiacFS and Storage Direction](#zodiacfs-and-storage-direction)
-   [Development Roadmap](#development-roadmap)
-   [Design Rules for Future
    Decisions](#design-rules-for-future-decisions)
-   [Current Development Context](#current-development-context)
-   [What ZodiacOS Is Not](#what-zodiacos-is-not)
-   [Project Commitment](#project-commitment)

------------------------------------------------------------------------

## Vision

ZodiacOS aims to be an open, user-controlled operating system built
around:

-   **Explicit authority:** processes and services receive only the
    access they need.
-   **Strong isolation:** a failure or compromise in one component
    should not automatically compromise the rest of the system.
-   **Modularity:** keep the trusted kernel core focused and use clear
    interfaces between components.
-   **User ownership:** users should be able to understand, configure,
    inspect, and control their machines.
-   **Compatibility:** support established software ecosystems where
    practical without allowing legacy requirements to define the entire
    native architecture.
-   **Transparency:** make the source code, interfaces, design
    decisions, and security practices open to inspection.
-   **Evolution:** allow the implementation to improve without
    abandoning the system's foundational principles.

The goal is not to make "another Unix" or "a more secure Linux." The
goal is to build a coherent operating system with its own philosophy and
engineering trade-offs.

------------------------------------------------------------------------

## The ZodiacOS Philosophy

### Freedom Through Structure. Security Through Design.

ZodiacOS is founded on the belief that freedom and security are not
opposites.

A computer should give its user meaningful control without giving every
application unlimited power. Security should be designed into the
architecture rather than added as a collection of optional features
after the system is built.

The central philosophy can be expressed in five commitments:

1.  **Power belongs to the user.**
2.  **Authority must be earned.**
3.  **Security must be enforced.**
4.  **Complexity must be contained.**
5.  **Knowledge must remain open.**

These commitments guide decisions about the kernel, memory, processes,
drivers, filesystems, applications, packages, and system services.

------------------------------------------------------------------------

## The Seven Principles

### 1. Security Is the Foundation, Not a Feature

Every component must be designed with the possibility that it can fail,
contain a vulnerability, or behave unexpectedly.

Security should influence the design of memory management, process
isolation, system calls, drivers, filesystems, package installation, and
application permissions from the beginning.

Design implications:

-   Isolate processes from one another.
-   Protect kernel memory from unprivileged applications.
-   Validate data and requests crossing privilege boundaries.
-   Keep privileged code limited and carefully reviewed.
-   Contain failures so one compromised component does not automatically
    compromise the entire system.
-   Make security-sensitive actions auditable where practical.

Rust can reduce many memory-safety risks, but it does not make an
operating system automatically secure. Unsafe code, hardware
interactions, authorization logic, concurrency, and service interfaces
still require careful design, testing, and review.

**Rule:** No component should possess more authority than its
responsibilities require.

### 2. The User Owns the Machine

The operating system exists to serve its user. Users should be able to
understand, configure, modify, and control their computing environment.

Design implications:

-   Avoid requiring a vendor account for basic local use.
-   Make telemetry transparent and controllable.
-   Give users clear control over application permissions.
-   Document system behavior and configuration.
-   Support inspection, modification, and redistribution according to
    the project's chosen open-source license.
-   Avoid unnecessary vendor-controlled restrictions.

User ownership does not mean every process should have unrestricted
access. The owner can choose strong security boundaries and still remain
in control of the machine.

**Rule:** The operating system exists to serve its user, not to control
its user.

### 3. Every Component Has Explicit Authority

Access to files, memory, devices, networking, and system services should
be deliberate and enforceable.

ZodiacOS should investigate capability-based security alongside familiar
identity- and permission-based controls. A capability can represent
narrowly scoped authority over a resource---for example, permission to
read a particular directory without granting permission to modify it or
access unrelated locations.

Design questions that must be answered include:

-   How are capabilities created and validated?
-   Can authority be delegated?
-   How is access revoked?
-   How are capabilities inherited by child processes?
-   How are privileged services protected?
-   How are permissions presented to users?

**Rule:** Access must be granted deliberately, limited to its purpose,
and enforced by the system.

### 4. Simplicity at the Core, Power at the Edges

The kernel should provide essential mechanisms while suitable
higher-level functionality lives in separate services or applications.

Potential kernel responsibilities include:

-   Memory and address-space management.
-   Scheduling and context switching.
-   Interrupt and exception handling.
-   Process and thread primitives.
-   System-call entry and validation.
-   Interprocess communication mechanisms.
-   Fundamental access-control enforcement.

As the system matures, suitable components---such as filesystem
implementations, networking services, graphics services, and
drivers---can be evaluated for isolation outside the kernel.

This is a direction, not a requirement to move every component into user
space immediately. Early development may reasonably keep some components
in kernel space until the necessary infrastructure exists.

**Rule:** Keep the trusted core focused, and make the rest modular where
practical.

### 5. Compatibility Must Preserve ZodiacOS's Identity

ZodiacOS should support established software ecosystems where practical,
but compatibility must not dictate every internal design decision.

The intended direction has three broad layers:

1.  **Native ZodiacOS applications:** software designed for ZodiacOS's
    native application model and security rules.
2.  **Portable interfaces and runtimes:** support for suitable
    standards, libraries, and cross-platform technologies.
3.  **Compatibility environments:** future support for selected
    applications from other operating systems, including Windows
    applications, where feasible.

Windows compatibility is a long-term engineering goal. Loading a Windows
PE executable alone is not sufficient: applications may also depend on
Windows APIs, runtime libraries, graphics interfaces, drivers, and
system services.

**Rule:** Support other ecosystems, but preserve the integrity of the
native system.

### 6. Openness Must Include the Right to Understand

Openness should mean more than publishing source code. Developers and
users should be able to understand how the system works and why
important decisions were made.

Goals include:

-   Public source code and documented build instructions.
-   Documented system calls, ABIs, and application interfaces.
-   Public architectural design decisions.
-   A responsible vulnerability-reporting process.
-   Independent review of security-critical components.
-   Reproducible builds where practical.
-   A transparent contribution and review process.

Open source does not automatically guarantee security. Security requires
implementation quality, testing, auditing, updates, and responsible
maintenance.

**Rule:** Trust should come from verifiable design and transparent
engineering, not secrecy alone.

### 7. The System Must Evolve Without Abandoning Its Principles

ZodiacOS must adapt to new hardware, software requirements, and security
threats.

Goals include:

-   Modular subsystems.
-   Carefully versioned interfaces.
-   Stable interfaces where compatibility is promised.
-   Explicitly experimental interfaces where change is expected.
-   Secure package and update mechanisms.
-   Documented migration and compatibility policies.

**Rule:** Improve the implementation without compromising the principles
that make the system trustworthy.

------------------------------------------------------------------------

## Architectural Direction

ZodiacOS is being developed as a bare-metal operating system using Rust
and Assembly. The current target is `x86_64-unknown-none`, with an
x86-64 boot path and QEMU-based development.

A microkernel-inspired or hybrid architecture is worth investigating
because service isolation aligns with the project's security goals.
However, the final kernel architecture has **not** been irrevocably
decided.

The choice should be based on a documented threat model, measured
performance, implementation complexity, and the quality of the isolation
boundaries---not on the assumption that one kernel architecture is
automatically more secure.

### Architectural goals

-   A small and carefully controlled privileged core.
-   Clear boundaries between kernel mode and user mode.
-   Separate process address spaces.
-   Controlled system calls and interprocess communication.
-   Explicit resource ownership and permission checks.
-   The ability to isolate suitable drivers and services.
-   Well-defined interfaces between kernel and user space.

### Important distinction

A modular kernel is not automatically a microkernel. A
microkernel-inspired design requires real isolation boundaries and
carefully designed communication mechanisms. ZodiacOS should describe
its architecture accurately as it evolves.

------------------------------------------------------------------------

## Security Model

Security is a foundational design objective, not a completed feature
set.

### Memory and process isolation

Planned mechanisms include:

-   Separate virtual address spaces for user processes.
-   Hardware-enforced page permissions.
-   Protection of kernel memory from user-mode access.
-   Controlled transitions between user mode and kernel mode.
-   Validation of pointers, buffers, and arguments crossing system-call
    boundaries.
-   Safe resource cleanup when processes terminate.

On x86-64, this requires correct use of page tables, privilege levels,
interrupt handling, and carefully controlled kernel-entry mechanisms.

### Least privilege and capabilities

Applications and services should receive only the authority they need.
ZodiacOS should explore capability-based access control and determine
how it works alongside users, groups, roles, and traditional
permissions.

### Driver and service isolation

Where technically practical, drivers and services should run with
reduced privileges and communicate with the kernel through narrow,
documented interfaces. Hardware access must remain controlled by the
kernel or another explicitly trusted mechanism.

### Auditing and transparency

Users and administrators should eventually be able to inspect important
security decisions, permission grants, and security-sensitive events.

### Security is not a single feature

No programming language, kernel architecture, signature scheme, or
permission system guarantees a secure operating system by itself.
ZodiacOS must use layered defenses, testing, code review, vulnerability
handling, and timely updates.

------------------------------------------------------------------------

## Applications and Compatibility

### Native application ecosystem

ZodiacOS should define its own native executable format and application
binary interface (ABI). The format and ABI should be documented and
versioned so third-party developers can build applications without
depending on undocumented kernel internals.

The native application model should integrate with the operating
system's permission and isolation model.

### ZPM and `.zpk` packages

The planned ZodiacOS Package Manager (ZPM) is intended to manage
applications and, as the architecture allows, drivers, plugins, and
other installable components.

The `.zpk` package format is a proposed package format for distributing
ZodiacOS software.

Long-term package-management goals:

-   Package metadata and dependency declarations.
-   Cryptographic package signatures and trusted publisher keys.
-   Integrity verification.
-   Controlled installation and removal.
-   Secure update handling and rollback considerations.
-   Clear permission declarations.
-   Separation between installing a package and granting it access to
    sensitive resources.
-   A documented process for third-party developers.

A valid signature can help establish package authenticity and integrity;
it does not prove that the software is harmless.

### Windows compatibility

Windows application compatibility is a long-term goal, not a current
capability. It should be developed as a compatibility subsystem rather
than by weakening the native security model.

------------------------------------------------------------------------

## ZodiacFS and Storage Direction

ZodiacOS has a planned custom filesystem direction called **ZodiacFS**.

The terminology established for the design is:

-   **Container:** a filesystem volume. A Container does not have to
    represent an entire physical drive; it may exist on a partition. A
    USB drive may also be formatted as a Container.
-   **Cluster:** a collection of related filesystem data, according to
    the ZodiacFS design.

The operating system should detect newly inserted Containers and notify
the user---for example, with a **"New Container Inserted"**
notification. Mounting, availability, and access should follow explicit
operating-system policies.

The precise on-disk format, metadata structures, journaling or recovery
strategy, integrity model, encryption design, and crash-consistency
guarantees still need to be specified and implemented. They should not
be treated as completed features until they exist and have been tested.

------------------------------------------------------------------------

## Development Roadmap

The following roadmap translates the philosophy into engineering work.
It is a suggested order; implementation priorities may change as the
kernel develops.

### Stage 1 --- Kernel foundations

-   [ ] Physical memory management.
-   [ ] Virtual memory management.
-   [ ] Page-level memory protection.
-   [ ] Interrupt and exception handling.
-   [ ] Scheduler and context switching.
-   [ ] Kernel threads.
-   [ ] Synchronization primitives.
-   [ ] Defined system-call entry and validation.
-   [ ] Clear kernel/user privilege boundaries.

### Stage 2 --- Process isolation

-   [ ] Process creation and termination.
-   [ ] Separate user address spaces.
-   [ ] User-mode execution.
-   [ ] Controlled system calls.
-   [ ] Interprocess communication.
-   [ ] Resource ownership and cleanup.
-   [ ] Robust handling of invalid user requests.

### Stage 3 --- Zodiac security model

-   [ ] Write a threat model and security architecture document.
-   [ ] Define the identity and permission model.
-   [ ] Design capability creation, delegation, inheritance, and
    revocation.
-   [ ] Implement least-privilege resource access.
-   [ ] Design application isolation.
-   [ ] Investigate driver isolation.
-   [ ] Add security-event auditing.
-   [ ] Establish vulnerability reporting and security review practices.

### Stage 4 --- Storage and services

-   [ ] Specify the ZodiacFS on-disk format.
-   [ ] Implement storage-device abstractions.
-   [ ] Implement Container discovery and lifecycle management.
-   [ ] Define mounting and access policies.
-   [ ] Design filesystem recovery and integrity behavior.
-   [ ] Evaluate which filesystem and device services can run outside
    the kernel.

### Stage 5 --- Native application ecosystem

-   [ ] Define the native executable format.
-   [ ] Define and document the application ABI.
-   [ ] Build a native application loader.
-   [ ] Define the application permission model.
-   [ ] Implement ZPM.
-   [ ] Specify and implement `.zpk` packages.
-   [ ] Add package signature and integrity verification.
-   [ ] Publish developer documentation and SDK tooling.

### Stage 6 --- User experience and compatibility

-   [ ] Framebuffer and graphics foundations.
-   [ ] Input-device support.
-   [ ] Windowing and desktop services.
-   [ ] User-facing permission management.
-   [ ] Networking services.
-   [ ] Portable application interfaces.
-   [ ] Investigate a Windows compatibility environment.

------------------------------------------------------------------------

## Design Rules for Future Decisions

When choosing between two implementations, evaluate them against these
questions:

1.  **User control:** Does this preserve the user's ability to
    understand and control the machine?
2.  **Least privilege:** Does each component receive only the authority
    it needs?
3.  **Isolation:** Can a failure or compromise be contained?
4.  **Trusted computing base:** Does this unnecessarily add complexity
    to privileged code?
5.  **Clarity:** Are interfaces and responsibilities well-defined?
6.  **Compatibility:** Does this preserve native ZodiacOS design while
    supporting useful standards?
7.  **Transparency:** Can the behavior and security implications be
    inspected and documented?
8.  **Maintainability:** Can the design be tested, updated, and evolved
    safely?

When principles conflict, document the trade-off rather than hiding it.

### An important open design decision

ZodiacOS has two goals that must be balanced:

-   **Security-first:** performance and compatibility must operate
    within strong security boundaries.
-   **Freedom-first:** users should have extensive control and be able
    to choose how strict their security policies are.

These goals can coexist, but the project still needs to define what
happens when they conflict. A promising direction is to make secure
defaults the baseline while giving knowledgeable users documented,
deliberate control over policy---without allowing ordinary applications
to bypass protections.

This is a design question to resolve through the threat model and
security architecture, not an already-finalized decision.

------------------------------------------------------------------------

## Current Development Context

The project has been developed as a bare-metal Rust and Assembly
operating system.

Known development context:

-   Kernel source entry point: `kernel/src/lib.rs`.
-   Target architecture: `x86_64-unknown-none`.
-   Custom x86-64 Assembly bootloader.
-   Development and testing with QEMU on macOS.
-   A basic kernel shell has been brought up.
-   Rust is used for kernel implementation, with Assembly for low-level
    boot and architecture-specific work.

These details describe the project's development context; they do not
imply that every roadmap item above is already implemented.

Build commands, bootloader layout, linker configuration, and exact setup
instructions should be documented separately and kept synchronized with
the repository as they change.

------------------------------------------------------------------------

## What ZodiacOS Is Not

-   It is not intended to be a Unix clone.
-   It is not intended to sacrifice security for compatibility.
-   It is not intended to give every application unrestricted access in
    the name of user freedom.
-   It does not assume that Rust alone makes a kernel secure.
-   It does not assume that a microkernel is automatically secure or
    that a monolithic kernel cannot be secure.
-   It does not claim that planned features are implemented before they
    are built, tested, and documented.

------------------------------------------------------------------------

## Project Commitment

ZodiacOS exists to build a computing environment where users retain
control, applications operate within clearly defined boundaries,
developers can innovate, and the system remains open to examination and
improvement.

Its implementation may evolve. Its interfaces may mature. Its
architecture may change as engineering experience grows.

But its foundational commitments should remain:

**Freedom Through Structure. Security Through Design.**

**Power belongs to the user. Authority must be earned. Security must be
enforced. Complexity must be contained. Knowledge must remain open.**

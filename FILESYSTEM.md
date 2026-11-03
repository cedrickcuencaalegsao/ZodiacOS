# ZodiacFS
### The Native Object-Centric Filesystem of ZodiacOS

**Project:** ZodiacOS  
**Component:** ZodiacFS  
**Architecture:** Object-Centric, Container-Based Filesystem  
**Implementation Language:** Rust  
**Initial Target:** x86_64  
**Environment:** Bare-metal, `no_std`  
**Status:** Proposed Design Specification

---

# 1. Introduction

ZodiacFS is a custom filesystem designed specifically for ZodiacOS.

Unlike traditional filesystems that primarily organize storage through hierarchical directories, ZodiacFS introduces an object-centric storage architecture built around Containers, Clusters, Packages, Folders, and Items.

Its purpose is to provide a different approach to organizing, storing, managing, protecting, and transferring data.

ZodiacFS is designed around these principles:

- **Object identity:** Every stored resource has a unique identity independent of its name.
- **Container-based storage:** Every formatted filesystem volume is represented as a Container.
- **Structured organization:** Data follows a defined organizational hierarchy.
- **Modularity:** Packages can be managed independently.
- **Application integration:** Applications are represented through Bundles.
- **Portability:** Capsules provide a way to transfer collections of Packages between devices.
- **Security:** Storage access is governed by explicit permissions and kernel-enforced security policies.
- **Data integrity:** Filesystem metadata and stored data should be protected against corruption.
- **Extensibility:** The format should support future capabilities without requiring a complete redesign.

ZodiacFS is not intended to be a direct copy of NTFS, ext4, APFS, or ZFS. It is an independent filesystem design that uses established storage-engineering techniques while introducing its own organizational concepts.

---

# 2. Core Architecture

ZodiacFS has five primary organizational levels:

```text
ZodiacFS
│
└── Container
    │
    └── Cluster
        │
        └── Package
            │
            ├── Folder
            │   └── Item
            │
            └── Item
```

The five levels have distinct responsibilities.

| Level | Component | Responsibility |
|---|---|---|
| 1 | Container | Represents a formatted filesystem volume. |
| 2 | Cluster | Groups related Packages. |
| 3 | Package | Contains Folders and Items. |
| 4 | Folder | Organizes Items within a Package. |
| 5 | Item | Represents an individual stored resource. |

**Important hierarchy rules:**

1. A Container contains Clusters.
2. A Cluster contains related Packages.
3. A Package contains Folders and may also contain Items directly.
4. A Folder contains Items only.
5. An Item cannot contain another Item, Folder, Package, or Cluster.
6. Packages cannot contain other Packages.
7. Folders cannot contain other Folders.
8. The hierarchy has a fixed maximum depth.

This avoids unlimited directory nesting and establishes predictable organizational boundaries.

---

# 3. Container

## 3.1 Definition

A Container represents an independently identifiable ZodiacFS storage volume.

A Container is not necessarily an entire physical drive.

It can represent a formatted partition or another supported logical storage region.

Examples include:

- Internal SSD partition.
- Secondary SSD partition.
- USB flash drive.
- External HDD.
- External SSD.
- Virtual disk image.

One physical drive can contain multiple partitions, with each partition potentially formatted as an independent ZodiacFS Container.

Example:

```text
Physical Storage
│
├── SSD 1 (1 TB)
│   │
│   ├── Partition 1
│   │   └── Container: ZodiacOS
│   │
│   └── Partition 2
│       └── Container: UserData
│
└── USB Drive (64 GB)
    │
    └── Partition 1
        └── Container: Transfer
```

Each Container maintains its own filesystem metadata, Clusters, Packages, object indexes, allocation structures, and data storage.

## 3.2 Container Responsibilities

The Container Manager is responsible for:

- Container creation.
- Container formatting.
- Container identification.
- Container validation.
- Container mounting.
- Container unmounting.
- Container recovery.
- Storage capacity management.
- Filesystem metadata management.
- Container lifecycle management.

## 3.3 Container Metadata

A proposed Container metadata structure:

```json
{
  "id": "container-001",
  "name": "ZodiacOS",
  "filesystem": "ZodiacFS",
  "version": 1,
  "volume_type": "partition",
  "capacity": 107374182400,
  "available_space": 85899345920,
  "status": "mounted"
}
```

The actual on-disk representation should use compact binary structures rather than JSON.

## 3.4 Container Creation and Formatting

Users should be able to create and format Containers through ZodiacOS.

Illustrative shell commands:

```bash
zfs device list
zfs partition list

zfs container create /dev/nvme0n1p2 --name UserData

zfs container format UserData

zfs container mount UserData
```

Formatting initializes the ZodiacFS filesystem structures on the selected storage region.

Formatting is destructive and must require appropriate user authorization and confirmation.

---

# 4. Container Detection and USB Integration

ZodiacOS should automatically detect newly connected storage devices.

When a USB drive or another supported storage device is inserted, the operating system should:

1. Detect the physical device.
2. Initialize the appropriate storage driver.
3. Read its partition table.
4. Identify available partitions.
5. Check for a ZodiacFS Container.
6. Validate the Container metadata.
7. Make the Container available for mounting.
8. Notify the user.

## 4.1 Container Insertion Notification

Example notification:

```text
ZodiacOS

New Container Inserted

Name: Transfer
Device: USB Storage
Capacity: 64 GB
Filesystem: ZodiacFS

Status: Recognized
```

The notification should allow the user to open or mount the Container.

## 4.2 Container Detection States

| State | Behavior |
|---|---|
| New ZodiacFS Container | Detect, validate, and offer mounting. |
| Recognized Container | Identify and make available. |
| Unknown filesystem | Offer compatible handling or formatting options. |
| Corrupted Container | Warn the user and offer recovery tools. |
| Unsupported filesystem | Report that it cannot currently be mounted. |
| Removed Container | Release resources and update device state. |

ZodiacOS must never automatically format an unknown USB drive.

## 4.3 Container Lifecycle

```text
Device Detected
      |
      v
Read Partition Table
      |
      v
Identify Container
      |
      v
Validate ZodiacFS
      |
      +----------------+
      |                |
    Valid            Invalid
      |                |
      v                v
 Available          Recovery
      |
      v
    Mounted
      |
      v
   Unmount
      |
      v
    Ejected
```

Before a device is physically removed, ZodiacOS should flush pending writes, release filesystem resources, and safely unmount its Container.

---

# 5. Cluster

## 5.1 Definition

A Cluster is a collection of related Packages.

It provides a higher organizational level for grouping Packages that belong to the same system component, purpose, or logical category.

A Cluster is not an application and is not a portable transfer archive.

Its primary responsibility is organizing related Packages within a Container.

## 5.2 Examples

### Development Cluster

```text
Cluster: Development
│
├── Package: Rust
├── Package: ZodiacSDK
├── Package: Compiler
└── Package: Debugger
```

### System Cluster

```text
Cluster: System
│
├── Package: Kernel
├── Package: Drivers
├── Package: Libraries
└── Package: SystemConfiguration
```

### User Cluster

```text
Cluster: Users
│
├── Package: CedrickDocuments
├── Package: CedrickMedia
└── Package: CedrickProjects
```

## 5.3 Cluster Metadata

A proposed structure:

```json
{
  "id": "cluster-001",
  "name": "Development",
  "type": "cluster",
  "packages": [
    "package-rust",
    "package-zodiacsdk",
    "package-compiler"
  ]
}
```

A Cluster may eventually have an owner, description, and optional security policy.

A Cluster is primarily an organizational unit. It should not automatically become a security boundary. Security isolation must be explicitly configured and enforced by the kernel.

---

# 6. Package

## 6.1 Definition

A Package is a collection of Folders and Items.

Packages are independently identifiable and manageable units within a Cluster.

A Package can contain Items directly or organize them into Folders.

Example:

```text
Package: Documents
│
├── Folder: Reports
│   ├── Item: report.pdf
│   └── Item: summary.txt
│
├── Folder: Research
│   └── Item: research.pdf
│
└── Item: notes.txt
```

## 6.2 Package Rules

- A Package belongs to a Cluster in the normal Container hierarchy.
- A Package can contain multiple Folders.
- A Package can contain Items directly.
- A Package cannot contain another Package.
- A Package cannot contain another Cluster.
- Packages must have unique identities.
- Packages can be installed, removed, exported, imported, or managed independently.

## 6.3 Package Metadata

```json
{
  "id": "package-001",
  "name": "Documents",
  "type": "package",
  "folders": [
    "folder-reports",
    "folder-research"
  ],
  "items": [
    "item-notes"
  ]
}
```

The Package Manager is responsible for managing Package registration, metadata, contents, and lifecycle operations.

---

# 7. Folder

## 7.1 Definition

A Folder organizes Items within a Package.

Unlike traditional directory structures, ZodiacFS does not support unlimited nested Folders.

A Folder contains Items only.

Example:

```text
Package: Projects
│
├── Folder: ZodiacOS
│   ├── Item: README.md
│   ├── Item: Cargo.toml
│   └── Item: kernel.rs
│
└── Folder: TensorLoom
    ├── Item: model.rs
    └── Item: training.rs
```

## 7.2 Folder Rules

- A Folder belongs to a Package.
- A Folder contains Items.
- A Folder cannot contain another Folder.
- A Folder cannot contain a Package.
- A Folder cannot contain a Cluster.
- Each Folder has a unique identity within the filesystem.

The fixed-depth design simplifies path resolution and metadata management.

---

# 8. Item

## 8.1 Definition

An Item is the individual resource stored inside a Package or Folder.

It is the lowest level of the ZodiacFS organizational hierarchy.

Examples:

- Documents.
- Images.
- Videos.
- Audio files.
- Executables.
- Drivers.
- Configuration files.
- Libraries.
- Application resources.
- Other supported binary data.

## 8.2 Object Identity

Every Item should have a unique identifier independent of its filename.

For example:

```json
{
  "id": "019A8F2B",
  "name": "report.pdf",
  "type": "file",
  "size": 8192,
  "parent": "folder-reports",
  "permissions": {
    "read": true,
    "write": true,
    "execute": false
  }
}
```

If the user renames the Item:

```text
Before:
document.txt
ID: 019A8F2B

After:
my_document.txt
ID: 019A8F2B
```

Its identity remains unchanged.

This makes it possible for applications and other filesystem components to reference Items by ID rather than relying exclusively on filenames or paths.

## 8.3 Item Metadata

An Item may have:

- Unique object ID.
- Name.
- Type.
- Size.
- Owner.
- Group.
- Permissions.
- Creation timestamp.
- Modification timestamp.
- Data location.
- Integrity checksum.
- Optional extended attributes.

The actual file contents are stored separately from the Item metadata.

---

# 9. Bundle

## 9.1 Definition

A Bundle represents an application and its associated resources.

Unlike a Cluster, which groups related Packages for organizational purposes, a Bundle describes an application as a complete logical unit.

An application may require multiple Packages.

Example:

```text
Bundle: ZodiacEditor
│
├── Package: Application
│   ├── Folder: Binary
│   │   └── Item: editor.zapp
│   └── Folder: Resources
│       └── Item: icon.png
│
├── Package: Libraries
│   └── Folder: Dependencies
│       └── Item: library.bin
│
└── Package: Configuration
    └── Folder: Defaults
        └── Item: settings.json
```

## 9.2 Bundle Manifest

A Bundle should have a manifest describing:

- Bundle ID.
- Application name.
- Version.
- Entry point.
- Referenced Packages.
- Dependencies.
- Required permissions.
- Application metadata.
- Compatibility requirements.

Example:

```json
{
  "id": "bundle-zodiac-editor",
  "name": "ZodiacEditor",
  "version": "1.0.0",
  "entry_point": "editor.zapp",
  "packages": [
    "package-application",
    "package-libraries",
    "package-configuration"
  ],
  "permissions": [
    "document.read",
    "document.write"
  ]
}
```

## 9.3 Bundle Architecture

A Bundle should be a separate manifest and application-level representation that references Packages.

It should not be another mandatory level in the Container hierarchy.

This allows Packages to be reused where appropriate without duplicating their contents.

The Bundle Manager will be responsible for application registration and coordination with ZodiacOS's application loader and security system.

---

# 10. Capsule

## 10.1 Definition

A Capsule is a portable collection of multiple Packages.

Unlike a Cluster, which groups related Packages, a Capsule can contain unrelated Packages.

Its primary purpose is data transfer between Containers and devices.

Capsules are especially useful for USB transfers.

Example:

```text
Capsule: MyTransfer
│
├── Package: Documents
│   └── Folder: Reports
│       └── Item: report.pdf
│
├── Package: Photos
│   └── Folder: Vacation
│       └── Item: beach.png
│
├── Package: Music
│   └── Folder: Songs
│       └── Item: track.mp3
│
└── Package: Projects
    └── Folder: ZodiacOS
        └── Item: source.zip
```

The Packages do not need to belong to the same Cluster or Bundle.

## 10.2 Capsule File Format

Proposed extension:

**`.zcaps`**

Example:

```text
MyTransfer.zcaps
```

A Capsule should be self-describing and portable.

It should not depend on the original Container's physical storage locations or indexes.

## 10.3 Capsule Features

- Multiple independent Packages.
- Capsule manifest.
- Unique Capsule ID.
- Package metadata.
- Optional compression.
- Optional encryption.
- Integrity checksums.
- Version information.
- Import and export support.
- Validation before importing.
- Preservation of logical identities where appropriate.

During import, ZodiacFS can preserve logical identities when safe or assign new IDs when necessary.

## 10.4 Capsule Commands

Illustrative commands:

```bash
zfs capsule create MyTransfer

zfs capsule add MyTransfer Documents

zfs capsule add MyTransfer Photos

zfs capsule export MyTransfer /usb

zfs capsule inspect /usb/MyTransfer.zcaps

zfs capsule import /usb/MyTransfer.zcaps
```

Capsules should be usable across compatible ZodiacFS installations.

A Capsule is a distribution format, not a new mandatory level in the Container hierarchy.

---

# 11. Filesystem Paths

ZodiacFS should provide readable paths for users and applications.

Proposed path format:

```text
container://<container>/<cluster>/<package>/<folder>/<item>
```

Example:

```text
container://main/Users/Documents/Reports/report.pdf
```

Another example:

```text
container://main/System/Kernel/Boot/kernel.bin
```

For an Item directly inside a Package:

```text
container://main/Users/Documents/notes.txt
```

Path resolution follows these steps:

1. Identify the Container.
2. Locate the Cluster.
3. Locate the Package.
4. Optionally locate the Folder.
5. Resolve the Item.
6. Retrieve its metadata.
7. Access its stored data after authorization.

The path is a human-readable addressing mechanism, not the fundamental identity of the Item.

Internally, ZodiacFS should use stable IDs and indexes.

---

# 12. Object-Centric Storage Architecture

ZodiacFS separates logical organization from physical storage.

A proposed disk layout:

```text
ZodiacFS Container
│
├── Container Header
│
├── Cluster Registry
│
├── Package Registry
│
├── Folder Registry
│
├── Item Metadata Table
│
├── Object Index
│
├── Name Index
│
├── Data Allocation Map
│
├── Integrity Metadata
│
└── Data Region
    │
    ├── Item Contents
    ├── Item Contents
    ├── Item Contents
    └── Item Contents
```

## 12.1 Container Header

Stores:

- Filesystem signature.
- Filesystem version.
- Container ID.
- Block size.
- Total capacity.
- Metadata locations.
- Filesystem state.
- Integrity information.

## 12.2 Object Index

Maps stable object IDs to metadata locations.

This avoids scanning the entire storage volume whenever an Item is requested.

## 12.3 Name Index

Allows efficient name-based lookups within a Package or Folder.

Names should be scoped to their organizational parent, allowing different Packages or Folders to contain Items with the same name.

## 12.4 Data Allocation Map

Tracks occupied and available storage blocks.

## 12.5 Data Region

Stores actual Item contents separately from their metadata.

An Item may occupy multiple non-contiguous storage extents.

This allows the filesystem to manage large files without requiring one enormous contiguous storage region.

---

# 13. Security Architecture

Security is a fundamental design objective of ZodiacFS.

The filesystem should provide the metadata and mechanisms necessary for the ZodiacOS kernel to enforce access restrictions.

## 13.1 Traditional Permissions

Support owner, group, and other permission classes.

Example:

```text
File: secret.txt

Owner: user_1000
Group: developers

Permissions:
    Owner: read, write
    Group: deny
    Others: deny
```

## 13.2 Access-Control Lists

Support more granular access policies for:

- Individual users.
- Groups.
- System services.
- Authorized applications.

## 13.3 Capability-Based Access

ZodiacOS should eventually support capability-based access.

For example, a text editor could receive permission to access a document selected by the user without automatically receiving access to every document on the Container.

## 13.4 Kernel Enforcement

The filesystem must not be trusted to enforce security by itself.

The kernel must validate access at the appropriate security boundary.

Applications must not bypass access controls by directly issuing storage requests.

## 13.5 Cluster Security Policies

Clusters may eventually support optional policies that apply to their Packages.

Examples:

- System Cluster: restricted modifications.
- Applications Cluster: controlled application access.
- Users Cluster: user-owned data.

A Cluster is not automatically a security boundary.

---

# 14. Reliability and Data Integrity

ZodiacFS should prioritize protecting filesystem consistency and user data.

## 14.1 Crash Consistency

The filesystem must be designed to recover from unexpected shutdowns and interrupted writes.

The initial implementation should use simple, carefully specified metadata transactions.

## 14.2 Copy-on-Write

A future version may introduce copy-on-write metadata updates.

Instead of immediately overwriting existing metadata, modified structures can be written to new locations before updating their references.

This provides a foundation for:

- Snapshots.
- Atomic metadata updates.
- Safer filesystem modifications.
- Versioned filesystem states.

## 14.3 Checksums

Metadata and eventually file data should have integrity checksums.

These help detect corruption but do not automatically repair damaged data.

## 14.4 Filesystem Checker

ZodiacFS should provide a checking utility to validate:

- Container metadata.
- Cluster and Package relationships.
- Folder and Item references.
- Object indexes.
- Allocation maps.
- Data extent references.
- Integrity checksums.

## 14.5 Recovery

A recovery mechanism should detect inconsistent filesystem structures and provide safe recovery operations.

---

# 15. Performance Architecture

ZodiacFS should support efficient access to large numbers of Items.

Potential techniques include:

- Object ID indexing.
- B-tree or B+ tree indexes.
- Metadata caching.
- Data block caching.
- Efficient block allocation.
- Batched metadata updates.
- Efficient directory and Package lookups.
- Asynchronous storage operations where supported.

The fixed organizational hierarchy may simplify path traversal, but performance must still be measured through real benchmarks.

---

# 16. Compression, Encryption, and Advanced Features

These capabilities should be introduced incrementally.

Potential future features:

1. Optional data compression.
2. Optional encryption.
3. Filesystem snapshots.
4. Atomic file replacement.
5. Storage quotas.
6. Extended attributes.
7. File integrity verification.
8. Read-only recovery mode.
9. Versioned filesystem states.
10. Secure deletion policies where supported by the underlying storage hardware.

Encryption must include proper key management rather than merely encrypting disk blocks.

Snapshots and copy-on-write should be designed together with allocation and recovery mechanisms.

---

# 17. Compatibility with Other Filesystems

ZodiacFS should remain the native filesystem of ZodiacOS.

However, ZodiacOS may eventually support other filesystem formats.

| Filesystem | Intended role |
|---|---|
| ZodiacFS | Native filesystem. |
| FAT32 | Boot compatibility and removable storage. |
| exFAT | Removable storage interoperability. |
| ext4 | Linux filesystem compatibility. |
| NTFS | Windows storage compatibility. |

These are separate filesystem drivers and should not change ZodiacFS's native organizational model.

NTFS compatibility does not automatically provide Windows application compatibility.

Running Windows applications requires executable loading, compatible system APIs, process behavior, and additional operating-system services.

---

# 18. ZodiacFS and ZPM Integration

ZodiacFS should work closely with the Zodiac Package Manager (ZPM).

ZPM manages software packages and applications, while ZodiacFS provides the storage structures.

A proposed application installation process:

1. Download the application package.
2. Verify its integrity and authenticity.
3. Validate its manifest.
4. Check compatibility with ZodiacOS.
5. Inspect dependencies and requested permissions.
6. Create or update the appropriate Package structures.
7. Store the application's Items.
8. Register the Bundle.
9. Make the application available to the user.

Uninstalling an application should remove its associated Package contents and Bundle registration without accidentally deleting unrelated shared resources or user data.

---

# 19. Kernel Implementation Architecture

ZodiacFS will be implemented in Rust and integrated into the existing ZodiacOS kernel.

Suggested module structure:

```text
kernel/
└── src/
    ├── lib.rs
    │
    ├── drivers/
    │   └── storage/
    │       ├── mod.rs
    │       ├── block.rs
    │       ├── nvme.rs
    │       ├── ahci.rs
    │       └── usb_storage.rs
    │
    ├── fs/
    │   ├── mod.rs
    │   ├── vfs.rs
    │   ├── path.rs
    │   ├── permissions.rs
    │   │
    │   └── zodiacfs/
    │       ├── mod.rs
    │       ├── superblock.rs
    │       ├── container.rs
    │       ├── cluster.rs
    │       ├── package.rs
    │       ├── folder.rs
    │       ├── item.rs
    │       ├── object_index.rs
    │       ├── name_index.rs
    │       ├── allocator.rs
    │       ├── read.rs
    │       ├── write.rs
    │       ├── transaction.rs
    │       ├── integrity.rs
    │       └── check.rs
    │
    ├── storage/
    │   ├── mod.rs
    │   └── container_manager.rs
    │
    └── shell/
```

Additional userspace tools or services can implement:

```text
zfs-tool
zfs-manager
zfs-capsule
zpm
```

These are proposed tools and commands, not existing implementations.

## 19.1 Module Responsibilities

| Module | Responsibility |
|---|---|
| `vfs.rs` | Common filesystem interface. |
| `path.rs` | Path parsing and resolution. |
| `permissions.rs` | Permission metadata and policy support. |
| `superblock.rs` | Container header management. |
| `container.rs` | Container metadata and operations. |
| `cluster.rs` | Cluster registration and management. |
| `package.rs` | Package registration and management. |
| `folder.rs` | Folder metadata and Item relationships. |
| `item.rs` | Item metadata and identity. |
| `object_index.rs` | Object ID lookups. |
| `name_index.rs` | Name-based lookups. |
| `allocator.rs` | Storage allocation. |
| `read.rs` | Data reading. |
| `write.rs` | Data writing. |
| `transaction.rs` | Consistent metadata updates. |
| `integrity.rs` | Checksums and validation. |
| `check.rs` | Filesystem consistency checking. |

The kernel-side implementation should support `no_std`.

Use `core` and introduce `alloc` when the kernel allocator is ready.

The disk-format definitions should be independent of runtime filesystem implementation wherever possible.

---

# 20. Development Roadmap

ZodiacFS should be developed incrementally.

## Phase 1 — Filesystem Foundation

- Define the binary disk format.
- Define Container headers.
- Define block size and allocation structures.
- Implement block-device abstraction.
- Implement Container formatting.
- Implement Container mounting.
- Implement basic metadata storage.

**Goal:** Create and mount a minimal ZodiacFS Container in QEMU.

## Phase 2 — Organizational Structure

- Implement Clusters.
- Implement Packages.
- Implement Folders.
- Implement Items.
- Implement stable object IDs.
- Implement object and name indexes.
- Implement path resolution.

**Goal:** Create and navigate the complete Container → Cluster → Package → Folder → Item structure.

## Phase 3 — Persistent File Operations

- Create Items.
- Read Items.
- Write Items.
- Rename Items.
- Delete Items.
- Create Folders.
- Delete Folders.
- Manage Packages and Clusters.
- Implement allocation and deallocation.

**Goal:** Store and retrieve actual files across reboots.

## Phase 4 — Security

- Implement ownership.
- Implement permission classes.
- Add access-control lists.
- Integrate kernel access validation.
- Establish application access policies.

**Goal:** Prevent unauthorized filesystem access.

## Phase 5 — Container Management

- Detect storage devices.
- Read partition tables.
- Identify ZodiacFS Containers.
- Format partitions.
- Mount and unmount Containers.
- Implement USB insertion notifications.
- Support safe device removal.

**Goal:** Manage internal and removable Containers.

## Phase 6 — Bundles and Capsules

- Define Bundle manifests.
- Implement Bundle registration.
- Support Package references.
- Define the `.zcaps` format.
- Implement Capsule creation.
- Implement Capsule inspection.
- Implement Capsule export.
- Implement Capsule import.
- Validate transferred data.

**Goal:** Integrate applications and portable data transfers.

## Phase 7 — Reliability and Advanced Features

- Implement crash recovery.
- Add metadata checksums.
- Add filesystem checking.
- Introduce transactional metadata updates.
- Investigate copy-on-write.
- Add snapshots.
- Add optional compression and encryption.

**Goal:** Build a reliable production-oriented filesystem.

---

# 21. Testing Strategy

ZodiacFS should initially be tested using disk images in QEMU.

Testing should cover:

- Container formatting.
- Container mounting.
- Metadata persistence.
- Package creation and removal.
- Folder creation and removal.
- Item creation and deletion.
- Object ID stability.
- Duplicate filenames in different parents.
- Path resolution.
- Storage allocation.
- Out-of-space handling.
- Permission enforcement.
- Unexpected shutdowns.
- Filesystem recovery.
- USB detection.
- Safe unmounting.
- Capsule export and import.
- Corrupted metadata.
- Invalid references.
- Large files.
- Large numbers of Items.

A host-side utility should eventually be developed to format, inspect, and validate ZodiacFS images on macOS and other development platforms.

Example:

```bash
zfs-tool format disk.img
zfs-tool inspect disk.img
zfs-tool check disk.img
```

This allows filesystem testing without booting the entire operating system.

Never experiment with an untested filesystem implementation on a drive containing important user data.

---

# 22. Official Terminology

The following terminology defines the proposed ZodiacFS architecture.

| Term | Official meaning |
|---|---|
| ZodiacFS | Native filesystem of ZodiacOS. |
| Container | Independently identifiable formatted storage volume. |
| Cluster | Collection of related Packages. |
| Package | Collection of Folders and Items. |
| Folder | Organization unit containing Items. |
| Item | Individual file or resource. |
| Bundle | Application representation referencing its required Packages. |
| Capsule | Portable collection of potentially unrelated Packages. |
| `.zcaps` | Proposed Capsule file extension. |
| ZVFS | Proposed Zodiac Virtual Filesystem interface. |
| ZPM | Zodiac Package Manager. |

## Important Distinctions

**Container vs. Cluster**

A Container represents a storage volume. A Cluster organizes related Packages within that volume.

**Cluster vs. Bundle**

A Cluster organizes related Packages. A Bundle describes an application and references the Packages needed by that application.

**Cluster vs. Capsule**

A Cluster groups related Packages within a Container. A Capsule packages potentially unrelated Packages for transfer.

**Package vs. Bundle**

A Package is a filesystem organizational unit. A Bundle is an application-level representation.

**Container vs. Physical Drive**

A physical drive provides storage. A partition defines a storage region. A Container gives that region a ZodiacFS identity and structure.

---

# 23. Design Philosophy

ZodiacFS introduces a different way to think about filesystem organization.

Instead of making an unlimited hierarchy of directories the foundation of the system, it establishes a fixed organizational structure with independent object identities.

Its design separates:

- Storage volumes.
- Logical organization.
- Individual resources.
- Application representation.
- Portable distribution.

This creates a foundation where applications, user data, system components, and removable storage can all be managed through a consistent model.

The architecture should remain modular, security-conscious, extensible, and understandable to developers.

## Guiding Principle

> Every resource has an identity. Every collection has a purpose. Every Container has a structure. Every operation must be authorized, and every committed change must leave the filesystem recoverable.

**ZodiacFS — The Native Object-Centric Filesystem of ZodiacOS.**

---

*Document status: Proposed architecture and development specification. Features described as future capabilities are design goals, not claims of existing implementation.*
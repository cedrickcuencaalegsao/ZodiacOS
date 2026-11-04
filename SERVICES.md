# ZodiacOS System Services Architecture

> **Purpose:** Define the recommended default services, their responsibilities, startup behavior, IPC architecture, concrete interface examples, and implementation roadmap for ZodiacOS.

## 1. Core Philosophy

ZodiacOS follows a service-oriented architecture inspired by Unix, Linux service management, and macOS Service Management, while maintaining its own design philosophy.

> **The kernel manages hardware and system resources. Services provide operating-system functionality. Applications consume those services through controlled interfaces.**

The kernel should remain small, secure, and independent of high-level functionality. Higher-level operating-system features should be implemented as independent services wherever practical.

Fundamental security protections must remain enforced by the kernel. Userspace services must never be the sole enforcement mechanism for critical security boundaries.

## 2. Service Architecture

ZodiacOS services communicate through controlled inter-process communication (IPC), rather than directly accessing one another's memory.

```text
+------------------------------------------------------+
|                    Applications                      |
|       Authorized access to system services           |
+------------------------------------------------------+
                         |
                         v
+------------------------------------------------------+
|              ZodiacOS Service Manager                |
|       Discovery | Lifecycle | Dependencies           |
+------------------------------------------------------+
                         |
                         v
+------------------------------------------------------+
|                  ZodiacOS IPC Layer                  |
|       Message passing | Permissions | Events         |
+------------------------------------------------------+
                         |
                         v
+------------------------------------------------------+
|                    ZodiacOS Kernel                   |
| Scheduler | Memory | IPC primitives | Interrupts     |
| Syscalls | Capabilities | Hardware abstractions      |
+------------------------------------------------------+
```

### Responsibilities

- **Service Manager:** Starts, stops, monitors, restarts, and coordinates services. Resolves dependencies and supports on-demand activation.
- **IPC:** Provides controlled communication and message passing between processes and services.
- **Kernel:** Enforces process isolation, memory protection, privilege boundaries, access controls, and the low-level primitives required by services.

### Architectural Benefits

- Fault isolation.
- Independent service restarts.
- Explicit permissions.
- Easier debugging and maintenance.
- Smaller kernel attack surface.
- Independent service development.
- Support for third-party services.

Services should be startable through:

1. Boot dependencies.
2. Hardware or system events.
3. IPC requests and on-demand activation.
4. Explicit user or administrator requests.

## 3. ZodiacOS IPC Architecture

IPC is one of the most important foundations of ZodiacOS. It allows independent services and applications to communicate without directly accessing one another's memory.

I recommend implementing a **message-based IPC system** with synchronous request/response communication and asynchronous event notifications.

### 3.1 IPC Communication Model

ZodiacOS should initially support these communication patterns:

| Communication | Description | Example |
|---|---|---|
| Request/Response | A process sends a request and waits for a response. | Application requests a file. |
| Event Notification | A service publishes an event to subscribers. | USB Container inserted. |
| Service-to-Service | One service communicates with another authorized service. | Mount service requests filesystem information. |
| Asynchronous Messaging | A process sends a message without waiting for immediate completion. | Package installation request. |
| Service Discovery | A process locates a registered service and obtains its endpoint. | Application discovers ZodiacFS. |

### 3.2 IPC Message Structure

Every IPC message should have a defined header and payload.

A proposed message structure:

```rust
#[repr(C)]
pub struct IpcMessageHeader {
    pub message_id: u64,
    pub sender_pid: u64,
    pub receiver_pid: u64,

    pub service_id: u32,
    pub opcode: u32,

    pub flags: u32,
    pub payload_size: u32,

    pub request_id: u64,
}

pub struct IpcMessage {
    pub header: IpcMessageHeader,
    pub payload: IpcPayload,
}
```

The payload can contain structured data associated with the requested operation.

For example:

```rust
pub enum IpcPayload {
    Empty,
    Bytes(Vec<u8>),
    Text(String),
    U64(u64),
    U32(u32),
}
```

This is a conceptual interface. The actual kernel ABI should use a carefully defined representation with fixed-width fields, explicit serialization, and bounded payloads. Kernel IPC structures should not depend on Rust's `String`, `Vec`, or other dynamically sized types across a process boundary.

### 3.3 IPC Message Types

Define message types so services can understand the purpose of each message.

```rust
#[repr(u32)]
pub enum IpcMessageType {
    Request = 0,
    Response = 1,
    Event = 2,
    Notification = 3,
    Error = 4,
}
```

Examples:

- `Request`: An application requests an operation.
- `Response`: A service returns the operation result.
- `Event`: A service broadcasts a system event.
- `Notification`: A service sends an informational message.
- `Error`: A service reports a failed operation.

### 3.4 Service Identification

Every registered service should have a unique service identifier.

Example:

```rust
#[repr(u32)]
pub enum ZodiacServiceId {
    Init = 1,
    ServiceManager = 2,
    Logger = 3,
    Security = 4,
    Filesystem = 5,
    Mount = 6,
    DeviceManager = 7,
    Network = 8,
    PackageManager = 9,
}
```

Applications should not need to know a service's process ID permanently. Instead, they should discover a service through the service manager and receive an authorized endpoint or service handle.

The service manager is responsible for maintaining the mapping between service names, service identifiers, process IDs, and IPC endpoints.

### 3.5 Service Discovery Interface

Example of discovering a service:

```rust
pub fn find_service(
    name: &str,
) -> Result<ServiceHandle, ServiceError>;
```

Usage:

```rust
let fs = zodiac::ipc::find_service("zodiac-fsd")?;

let response = fs.request(
    FilesystemRequest::GetInfo {
        path: "/system",
    }
)?;
```

The returned handle should represent an authorized communication endpoint, not unrestricted access to another process.

### 3.6 Request and Response Interfaces

Services should expose typed operations instead of accepting arbitrary commands.

Example filesystem interface:

```rust
pub enum FilesystemRequest {
    Open {
        path: String,
        flags: u32,
    },

    Read {
        file_id: u64,
        size: usize,
    },

    Write {
        file_id: u64,
        data: Vec<u8>,
    },

    Close {
        file_id: u64,
    },

    GetInfo {
        path: String,
    },
}
```

Response:

```rust
pub enum FilesystemResponse {
    Opened {
        file_id: u64,
    },

    Data {
        bytes: Vec<u8>,
    },

    Written {
        bytes_written: usize,
    },

    Closed,

    Info {
        size: u64,
        is_directory: bool,
    },
}
```

These Rust enums describe the logical interface. Production IPC should serialize them into a stable, versioned wire format and validate all received data.

### 3.7 Example: Application Requests a File

```text
Application
    |
    | OPEN("/system/config.toml")
    |
    v
Zodiac IPC
    |
    | Validate endpoint and permissions
    |
    v
zodiac-fsd
    |
    | Check filesystem access
    | Resolve requested path
    | Open file
    |
    v
Zodiac IPC
    |
    | OPENED(file_id = 1042)
    |
    v
Application
```

The application does not directly access filesystem memory or storage hardware. It requests access through the authorized filesystem interface.

### 3.8 Example: USB Container Insertion Event

The device manager should publish events when hardware appears or disappears.

Example event:

```rust
pub enum DeviceEvent {
    DeviceConnected {
        device_id: u64,
        device_class: u32,
    },

    DeviceDisconnected {
        device_id: u64,
    },

    StorageDetected {
        device_id: u64,
        capacity: u64,
    },
}
```

Example event flow:

```text
USB Hardware
     |
     v
Kernel Device Detection
     |
     v
zodiac-deviced
     |
     | DeviceConnected
     v
zodiac-eventd
     |
     +--------------------+
     |                    |
     v                    v
zodiac-driverd       zodiac-usbd
     |                    |
     +---------+----------+
               |
               v
         zodiac-volume
               |
               v
           zodiac-fsd
               |
               v
          zodiac-mountd
               |
               v
          zodiac-notify
               |
               v
       "New Container Inserted"
```

Services subscribe only to events relevant to their responsibilities.

### 3.9 Example: Service Registration

When a service starts, it should register itself with the service manager.

Conceptual interface:

```rust
pub struct ServiceDescriptor {
    pub name: &'static str,
    pub version: u32,
    pub process_id: u64,
    pub endpoint_id: u64,
}

pub fn register_service(
    descriptor: ServiceDescriptor,
) -> Result<(), ServiceError>;
```

Example:

```rust
register_service(ServiceDescriptor {
    name: "zodiac-fsd",
    version: 1,
    process_id: current_process_id(),
    endpoint_id: current_endpoint_id(),
})?;
```

In a production implementation, service registration must be restricted to authorized processes. A service must not be able to impersonate another service by simply supplying its name.

### 3.10 Example: Permission-Controlled IPC

Every IPC request should be validated before a service performs a privileged operation.

For example:

```rust
pub enum Capability {
    FilesystemRead,
    FilesystemWrite,
    DeviceAccess,
    NetworkAccess,
    ProcessManagement,
    ServiceManagement,
}
```

Conceptual permission check:

```rust
pub fn authorize(
    process: ProcessId,
    capability: Capability,
) -> Result<(), SecurityError>;
```

Usage:

```rust
authorize(
    current_process_id(),
    Capability::FilesystemWrite,
)?;

filesystem.write(file_id, data)?;
```

The kernel must enforce capability ownership and access restrictions. A userspace authorization helper alone is not a security boundary.

For IPC requests, the kernel or trusted IPC infrastructure should validate:

- Sender identity.
- Receiver endpoint.
- Endpoint ownership.
- Capability authorization.
- Message size.
- Payload validity.
- Request ownership.
- Access to transferred memory or handles.

### 3.11 IPC Error Handling

Use consistent errors across services.

```rust
#[repr(u32)]
pub enum IpcError {
    InvalidEndpoint = 1,
    PermissionDenied = 2,
    ServiceUnavailable = 3,
    InvalidMessage = 4,
    MessageTooLarge = 5,
    Timeout = 6,
    ProcessNotFound = 7,
    OutOfMemory = 8,
    InternalError = 9,
}
```

Example:

```rust
match filesystem.open("/system/config.toml") {
    Ok(file) => {
        // Continue processing.
    }

    Err(IpcError::PermissionDenied) => {
        // Access denied.
    }

    Err(IpcError::ServiceUnavailable) => {
        // Retry or report service failure.
    }

    Err(_) => {
        // Handle other errors.
    }
}
```

Services should return explicit errors instead of silently failing.

### 3.12 IPC Security Requirements

The IPC subsystem must enforce:

1. Process isolation.
2. Endpoint ownership.
3. Capability-based access control.
4. Message-size limits.
5. Payload validation.
6. Protection against unauthorized service impersonation.
7. Safe memory transfer between processes.
8. Request and response correlation.
9. Timeout and cancellation support.
10. Protection against resource exhaustion and message flooding.

For larger payloads, such as file data, ZodiacOS can eventually support controlled shared-memory regions or memory-backed message transfer. These regions must have explicit ownership, permissions, and lifetime rules.

### 3.13 Recommended IPC Implementation Order

Implement IPC incrementally:

1. Process-to-process message passing.
2. Endpoint creation and destruction.
3. Sender and receiver identification.
4. Request and response messaging.
5. Service registration and discovery.
6. Message queues.
7. Event subscriptions and notifications.
8. Timeout and cancellation.
9. Capability-based IPC authorization.
10. Shared-memory transfer.
11. IPC monitoring and auditing.

Do not begin with complex shared-memory IPC. First establish a reliable, bounded message-passing system.

## 4. Default System Services

The following service names are proposed conventions and can be refined as implementation progresses.

### A. Core System Services

| Service | Responsibility |
|---|---|
| `zodiac-init` | First userspace process; initializes userspace and begins service startup. |
| `zodiac-service-manager` | Starts, stops, monitors, and restarts services; manages dependencies and lifecycle. |
| `zodiac-logger` | Centralized system logging. |
| `zodiac-config` | System configuration management. |
| `zodiac-eventd` | System-wide event notification and distribution. |
| `zodiac-watchdog` | Detects unresponsive system components and services. |
| `zodiac-time` | System clock and time management. |

The service manager must understand dependencies. For example, networking services should not start until the required network devices and drivers are ready.

### B. Memory and Security Services

| Service | Responsibility |
|---|---|
| `zodiac-securityd` | Central security policy coordination and monitoring. |
| `zodiac-authd` | User authentication. |
| `zodiac-capabilityd` | Capability and permission management. |
| `zodiac-keyd` | Cryptographic key management. |
| `zodiac-auditd` | Security auditing and recording security-relevant events. |
| `zodiac-memd` | Memory pressure monitoring. |
| `zodiac-oomd` | Out-of-memory handling and response. |

**Kernel-versus-service boundary:** The kernel must enforce page-table isolation, process privilege boundaries, memory access restrictions, and low-level memory-management rules. Userspace services can coordinate policies, authentication, auditing, and monitoring, but cannot replace kernel enforcement.

### C. Filesystem and Storage Services

| Service | Responsibility |
|---|---|
| `zodiac-fsd` | Filesystem management and filesystem operations coordination. |
| `zodiac-mountd` | Mounts and unmounts filesystem Containers. |
| `zodiac-volume` | Detects and manages storage volumes. |
| `zodiac-cache` | Filesystem caching. |
| `zodiac-indexd` | File indexing and search. |
| `zodiac-trash` | Deleted-file and trash management. |
| `zodiac-fscheck` | Filesystem integrity checking. |

#### ZodiacFS Terminology

- **Container:** A filesystem volume. A Container does not necessarily represent an entire physical drive. It may occupy a partition or another supported storage region. USB drives can also be formatted as Containers.
- **Cluster:** A collection of related filesystem data.
- **Mounting:** Making a Container available to the operating system and authorized users or applications.

#### USB Container Insertion Workflow

1. The kernel detects the hardware event.
2. `zodiac-deviced` identifies and registers the device.
3. `zodiac-driverd` locates and initializes an appropriate driver.
4. `zodiac-usbd` manages USB-specific device initialization.
5. `zodiac-volume` checks for storage volumes and supported Containers.
6. `zodiac-fsd` identifies and prepares the supported filesystem.
7. `zodiac-mountd` mounts the Container if permitted by policy.
8. The desktop notification service announces that a new Container has been inserted.

The user-facing experience should support a notification such as **"New Container Inserted."** Mounting and availability must follow OS security policy.

### D. Hardware and Device Services

| Service | Responsibility |
|---|---|
| `zodiac-deviced` | Device discovery and registration. |
| `zodiac-driverd` | Driver discovery, loading, management, and lifecycle. |
| `zodiac-pcid` | PCI and PCIe device discovery. |
| `zodiac-usbd` | USB device management. |
| `zodiac-inputd` | Keyboard, mouse, and other input handling. |
| `zodiac-audiod` | Audio device management. |
| `zodiac-displayd` | Display and graphics device management. |
| `zodiac-powermd` | Power management. |

Device discovery should be event-driven. When a device appears, the device manager publishes an event that interested services can consume.

`zodiac-inputd` may initially serve both system-level and desktop input needs. Its responsibilities can be separated later if necessary.

### E. Networking Services

| Service | Responsibility |
|---|---|
| `zodiac-netd` | Network interface management and coordination. |
| `zodiac-dhcpd` | DHCP client and network address configuration. |
| `zodiac-dnsd` | DNS resolution. |
| `zodiac-firewall` | Network filtering policy and firewall management. |
| `zodiac-tlsd` | Shared TLS infrastructure. |
| `zodiac-bluetoothd` | Bluetooth management. |
| `zodiac-wifid` | Wi-Fi management. |

The kernel should provide low-level networking primitives. Higher-level network configuration, connection management, and policy should be handled by services.

### F. User and Desktop Services

| Service | Responsibility |
|---|---|
| `zodiac-sessiond` | User session management. |
| `zodiac-login` | Login screen and authentication interface. |
| `zodiac-compositor` | Window composition and graphical output composition. |
| `zodiac-windowd` | Window management. |
| `zodiac-notify` | Desktop notifications. |
| `zodiac-clipboard` | Clipboard management. |
| `zodiac-desktop` | Desktop environment and shell experience. |
| `zodiac-accessibility` | Accessibility features and assistive interfaces. |

The compositor and window manager may initially be implemented as one process. They can be split into separate services when the GUI architecture becomes more mature.

### G. Application and Package Services

| Service | Responsibility |
|---|---|
| `zodiac-pkgd` | Package installation, removal, and package operations. |
| `zodiac-appd` | Application lifecycle management. |
| `zodiac-updated` | System and application update coordination. |
| `zodiac-repod` | Package repository management. |
| `zodiac-sandboxd` | Application isolation and sandbox coordination. |
| `zodiac-permissiond` | Application permission requests and user-facing permission management. |

ZPM and its supporting services should support:

- Installation and removal of `.zpk` packages.
- Dependency resolution.
- Package signature verification.
- Application updates.
- Driver and plugin installation.
- Package rollback.
- Uninstallation and cleanup.
- Controlled access to system resources.

Applications must not automatically receive unrestricted access to the filesystem, memory, devices, or network.

### H. Maintenance, Monitoring, and Recovery Services

| Service | Responsibility |
|---|---|
| `zodiac-healthd` | System health monitoring. |
| `zodiac-crashd` | Crash reporting and crash information collection. |
| `zodiac-backupd` | Backup management. |
| `zodiac-upgraded` | OS upgrade orchestration. |
| `zodiac-perfd` | Performance monitoring. |
| `zodiac-taskd` | Scheduled background tasks. |

Most maintenance services can be started on demand or according to a schedule rather than running continuously.

## 5. Boot Sequence

ZodiacOS should use staged boot initialization. Not every service needs to start at boot.

```text
Bootloader
   |
   v
Kernel Initialization
   |  CPU, memory, interrupts, scheduler, hardware abstractions
   v
Core Initialization
   |  Init, service manager, essential IPC mechanisms
   v
Security Initialization
   |  Policies, capabilities, auditing infrastructure
   v
Storage Initialization
   |  Device discovery, ZodiacFS, Containers and mounting
   v
System Services
   |  Logging, device management, networking, configured services
   v
User Session
   |  Authentication and (for graphical boot) desktop environment
   v
System Ready
      Accepts user applications
```

### Boot Stages

1. **Bootloader:** Loads the kernel and transfers control to it.
2. **Kernel initialization:** Initializes CPU facilities, memory management, interrupts, scheduler, and hardware abstractions.
3. **Core initialization:** Initializes the first userspace process, service manager, and essential IPC mechanisms.
4. **Security initialization:** Loads security policies and initializes capability and audit infrastructure.
5. **Storage initialization:** Discovers storage devices and initializes ZodiacFS and Container management.
6. **System services:** Starts configured services such as logging, device management, and networking when their dependencies are ready.
7. **User session:** Starts authentication and, for a graphical installation, the desktop environment.
8. **System ready:** ZodiacOS is ready to accept user applications.

## 6. Recommended Default Startup Policy

Do not start every available service during boot. Starting dozens of unnecessary services wastes memory, increases startup work, and expands the attack surface.

Use these activation policies:

- **Always:** Required core infrastructure.
- **When available:** Start when compatible hardware or a required resource exists.
- **Graphical boot:** Start when the graphical environment is requested.
- **On demand:** Start when a user, application, or another service requests it.
- **Scheduled:** Start when a scheduled maintenance task is due.

### Proposed Default Startup Table

| Service | Startup Policy |
|---|---|
| `zodiac-init` | Always |
| `zodiac-service-manager` | Always |
| `zodiac-logger` | Always |
| `zodiac-securityd` | Always |
| `zodiac-capabilityd` | Always |
| `zodiac-auditd` | Always |
| `zodiac-deviced` | Always |
| `zodiac-fsd` | Always |
| `zodiac-mountd` | Always |
| `zodiac-time` | Always |
| `zodiac-netd` | When networking is available |
| `zodiac-indexd` | On demand |
| `zodiac-bluetoothd` | When compatible hardware is available |
| `zodiac-audiod` | When compatible hardware is available |
| `zodiac-compositor` | Graphical boot |
| `zodiac-pkgd` | On demand |
| `zodiac-backupd` | On demand |
| `zodiac-updated` | On demand or scheduled |

This is a proposed policy, not a requirement that every service be implemented immediately.

## 7. Three Signature ZodiacOS Capabilities

### 7.1 Zodiac Guardian

A unified security service that coordinates and monitors:

- Application permissions.
- Security events.
- Suspicious activity.
- Policy violations.
- Security auditing and reporting.

Zodiac Guardian must work alongside kernel-enforced memory protection, process isolation, and capability checks. It must not replace kernel protections or become a single userspace point of failure for fundamental security enforcement.

### 7.2 Zodiac Storage Intelligence

A unified storage-management experience for ZodiacFS that coordinates:

- Containers.
- Clusters.
- Filesystem indexing and search.
- Automatic storage and device detection.
- Filesystem integrity checking.
- Storage health.
- Container availability and mounting.

It should provide a consistent experience for internal storage, partitions, removable drives, and supported USB Containers.

### 7.3 Zodiac Service Intelligence

A service manager and lifecycle architecture that:

- Understands service dependencies.
- Monitors service health.
- Starts services on demand.
- Restarts eligible failed services.
- Isolates service failures.
- Responds to hardware and system events.
- Supports explicit service permissions.

This makes service behavior predictable and helps keep the OS modular.

## 8. Implementation Roadmap

Build the service infrastructure incrementally. Do not attempt to implement every service at once.

| Phase | Focus | Services to Implement |
|---|---|---|
| 1 | Current kernel and userspace foundation | `zodiac-init`, `zodiac-service-manager`, `zodiac-logger` |
| 2 | Security infrastructure | `zodiac-securityd`, `zodiac-capabilityd`, `zodiac-auditd` |
| 3 | Storage and ZodiacFS | `zodiac-fsd`, `zodiac-mountd`, `zodiac-volume` |
| 4 | Hardware and devices | `zodiac-deviced`, `zodiac-driverd`, `zodiac-inputd` |
| 5 | Networking | `zodiac-netd`, `zodiac-dnsd`, `zodiac-firewall` |
| 6 | Application ecosystem | `zodiac-pkgd`, `zodiac-appd`, `zodiac-sandboxd` |
| 7 | Graphical environment | `zodiac-sessiond`, `zodiac-compositor`, `zodiac-desktop` |

### Recommended Immediate Milestone

The next major userspace milestone should be **`zodiac-service-manager`**.

Once the service manager exists, other OS functionality can be implemented as independent services instead of continuously expanding the kernel.

The first implementation should focus on:

1. A defined service executable and metadata format.
2. Service registration and discovery.
3. Start and stop operations.
4. Service dependency declarations and startup ordering.
5. Exit-status and health monitoring.
6. Controlled restart policies.
7. IPC-based service communication.
8. Permission checks for service operations.
9. Logging of service lifecycle events.
10. On-demand service activation.

## 9. Design Principles and Security Requirements

All ZodiacOS services should follow these principles:

1. **Keep the kernel focused:** Put high-level functionality in userspace when practical.
2. **Least privilege:** Give each service only the permissions and resources it needs.
3. **Isolation:** A service should not be able to freely read or modify another service's memory.
4. **Controlled IPC:** Services communicate through explicit interfaces and authorized messages.
5. **Kernel-enforced security:** Critical memory, privilege, and resource protections must be enforced by the kernel.
6. **Dependency-aware startup:** Start services only after their dependencies are ready.
7. **Event-driven hardware management:** Publish events when devices appear or disappear.
8. **Failure containment:** Monitor services and recover from eligible failures without crashing the entire OS.
9. **On-demand activation:** Avoid running unnecessary services.
10. **Auditable behavior:** Record important service and security events.
11. **Modularity:** Allow services to be developed, tested, updated, and restarted independently.
12. **Extensibility:** Provide a controlled path for third-party services, drivers, and applications.

## 10. References and Architectural Inspiration

The design takes inspiration from established operating-system service concepts:

- **Unix:** Small, composable tools and clear interfaces.
- **Linux systemd:** Service lifecycle management, dependencies, and staged startup.
- **macOS Service Management:** Managed background services and application integration.

These are architectural inspirations, not requirements to copy their implementations. ZodiacOS should define its own service protocols, metadata, security rules, IPC wire format, and lifecycle behavior.

---

**ZodiacOS principle:** The kernel protects and manages the machine. The service layer organizes the operating system. Applications operate through controlled capabilities and interfaces.
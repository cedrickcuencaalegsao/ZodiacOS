pub mod devfs;
pub mod file;
pub mod initramfs;
pub mod inode;
pub mod mount;
pub mod path;
pub mod procfs;
pub mod ramfs;
pub mod vfs;
pub mod zodiacfs; // native filesystem: Container > Cluster > Package > Folder > Item

pub use inode::{FileType, Stat};

use crate::{log_info, log_warn};
use alloc::{boxed::Box, format, string::String};
use errors::Error;

/// Largest root RAM disk, in 512-byte sectors (64 MiB). The disk is sparse: this is
/// a size limit, memory is only used for data actually stored.
const ROOT_DISK_SECTORS: usize = 128 * 1024;

/// Where the persistent data Container (second disk, hdb) is mounted.
pub const DATA_MOUNT: &str = "/Volumes/data";
const DATA_DRIVE: u8 = 1;
const DATA_DEVICE: &str = "hdb";

/// stdin/stdout/stderr of every process (used by file.rs).
pub const CONSOLE_PATH: &str = "/System/Devices/console";

/// The root Container layout, parents before children. The depth of a path
/// decides what it is: 1 = Cluster, 2 = Package, 3 = Folder.
const LAYOUT: &[(&str, u16)] = &[
    // System: operating-system components
    ("/System", 0o755),
    ("/System/Kernel", 0o755),
    ("/System/Kernel/Boot", 0o755),
    ("/System/Drivers", 0o755),
    ("/System/Libraries", 0o755),
    ("/System/Shared", 0o755),
    ("/System/Configuration", 0o755),
    ("/System/Configuration/zodiac", 0o755),
    ("/System/Configuration/zpm", 0o755),
    ("/System/Devices", 0o755), // devfs is mounted over this Package
    ("/System/Runtime", 0o755), // procfs is mounted over this Package
    ("/System/Temporary", 0o755),
    ("/System/Logs", 0o755),
    // Users: one Package per user
    ("/Users", 0o755),
    ("/Users/root", 0o700),
    // Applications: Core holds the built-in programs, one Package per Bundle after that
    ("/Applications", 0o755),
    ("/Applications/Core", 0o755),
    // Software: ZPM working data
    ("/Software", 0o755),
    ("/Software/Cache", 0o755),
    ("/Software/Database", 0o755),
    ("/Software/Store", 0o755),
    // Volumes: where other Containers are mounted
    ("/Volumes", 0o755),
    ("/Volumes/data", 0o755),
];

/// What a path is, in ZodiacFS terms.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Container,
    Cluster,
    Package,
    Folder,
    Item,
}

impl Level {
    pub fn name(self) -> &'static str {
        match self {
            Level::Container => "Container",
            Level::Cluster => "Cluster",
            Level::Package => "Package",
            Level::Folder => "Folder",
            Level::Item => "Item",
        }
    }

    /// VGA colour attribute used by the shell.
    pub fn color(self) -> u8 {
        match self {
            Level::Container => 0x0A, // green
            Level::Cluster => 0x0D,   // magenta
            Level::Package => 0x0E,   // yellow
            Level::Folder => 0x0B,    // cyan
            Level::Item => 0x0F,      // white
        }
    }
}

fn components(path: &str) -> usize {
    path.split('/').filter(|c| !c.is_empty()).count()
}

/// Classifies a canonical absolute path by its depth inside the innermost
/// ZodiacFS Container that covers it. Virtual mounts (devfs, procfs) sit on
/// top of Packages, so they count as part of the Container underneath.
pub fn level_of(abs: &str, kind: FileType) -> Level {
    let mut base = 0;
    let mut best = 0;
    for (p, fstype, _) in mount::list() {
        let covers = p == "/"
            || abs == p
            || (abs.starts_with(p.as_str()) && abs.as_bytes().get(p.len()) == Some(&b'/'));
        if fstype == "zodiacfs" && covers && p.len() >= best {
            best = p.len();
            base = components(&p);
        }
    }
    match components(abs).saturating_sub(base) {
        0 => Level::Container,
        1 => Level::Cluster,
        2 => Level::Package,
        3 if kind == FileType::Directory => Level::Folder,
        _ => Level::Item,
    }
}

/// The Container that holds `abs`: its name (the label it was formatted with) and
/// the rest of the path inside it, e.g. "/Applications/Core" -> ("ZodiacOS", "/Applications/Core").
pub fn container_of(abs: &str) -> (String, String) {
    let mut best: Option<(String, String)> = None; // (mount path, device)
    for (p, fstype, device) in mount::list() {
        let covers = p == "/"
            || abs == p
            || (abs.starts_with(p.as_str()) && abs.as_bytes().get(p.len()) == Some(&b'/'));
        if fstype == "zodiacfs"
            && covers
            && best.as_ref().map_or(true, |(bp, _)| p.len() >= bp.len())
        {
            best = Some((p, String::from(device)));
        }
    }
    let (mount_path, device) = best.unwrap_or((String::from("/"), String::from("?")));
    let name = vfs::resolve(&mount_path)
        .ok()
        .and_then(|node| node.label())
        .unwrap_or(device);
    let rest = if mount_path == "/" {
        abs
    } else {
        &abs[mount_path.len()..]
    };
    (name, String::from(rest))
}

/// The current user's home Package.
pub fn home_dir() -> String {
    let user = crate::user::get();
    format!("/Users/{}", crate::user::as_str(&user))
}

fn data_disk() -> Result<zodiacfs::block::DriveDisk, Error> {
    let sectors = crate::ata::identify(DATA_DRIVE)
        .ok_or(Error::NotFound)?
        .sectors as u64;
    zodiacfs::block::DriveDisk::open(DATA_DEVICE, sectors).ok_or(Error::NotFound)
}

/// Validates the ZodiacFS Container on the data disk and mounts it.
/// Never formats: an unformatted disk is reported, not touched.
pub fn mount_data() -> Result<(), Error> {
    let root = zodiacfs::mount(Box::new(data_disk()?))?;
    mount::mount(DATA_MOUNT, "zodiacfs", DATA_DEVICE, root)
}

/// Formats the data disk as a new Container and mounts it.
/// DESTRUCTIVE: the shell command must ask the user to confirm first.
pub fn format_data(label: &str) -> Result<(), Error> {
    if mount::is_mountpoint(DATA_MOUNT) {
        return Err(Error::Busy); // unmount first
    }
    zodiacfs::format(&data_disk()?, label)?;
    mount_data()
}

/// Sizes the root RAM disk: at most ROOT_DISK_SECTORS, at most half of the free
/// kernel heap (the rest is for the initramfs buffer and bookkeeping), at least 2 MiB.
fn root_disk_sectors() -> usize {
    let heap = crate::memory::allocator::size();
    let free = heap.saturating_sub(crate::memory::allocator::used());
    let sectors = (free / 2 / 512) & !7; // whole 4 KiB blocks
    let sectors = sectors.min(ROOT_DISK_SECTORS).max(4 * 1024);
    log_info!(
        "filesystem: heap {} KiB, free {} KiB, root disk {} KiB",
        heap / 1024,
        free / 1024,
        sectors / 2
    );
    sectors
}

/// Builds the root Container on a RAM disk, then mounts devices and runtime info.
pub fn init() -> Result<(), Error> {
    let disk = zodiacfs::block::MemDisk::new(root_disk_sectors())?;
    zodiacfs::format(&disk, "ZodiacOS")?;
    let root = zodiacfs::mount(Box::new(disk))?;
    mount::mount("/", "zodiacfs", "ramdisk", root)?;

    for &(path, mode) in LAYOUT {
        match vfs::create(path, FileType::Directory, mode) {
            Ok(_) | Err(Error::AlreadyExists) => {}
            Err(e) => {
                log_warn!("filesystem: cannot create {}: {}", path, e);
                return Err(e);
            }
        }
    }

    // Devices and runtime information appear as Items inside two ordinary Packages.
    mount::mount("/System/Devices", "devfs", "devfs", devfs::new_root())?;
    mount::mount("/System/Runtime", "procfs", "procfs", procfs::new_root())?;
    mount::mount("/System/Temporary", "ramfs", "ramfs", ramfs::new_root())?;

    match initramfs::load() {
        Ok(count) => log_info!("initramfs: {} entries unpacked", count),
        Err(e) => {
            log_warn!("initramfs: {}", e);
            return Err(e);
        }
    }

    // The data disk is optional: boot continues without it.
    match mount_data() {
        Ok(()) => log_info!("zodiacfs: data container mounted at {}", DATA_MOUNT),
        Err(Error::NotFound) => {
            log_warn!(
                "zodiacfs: {} has no ZodiacFS container (run the format command)",
                DATA_DEVICE
            )
        }
        Err(e) => log_warn!("zodiacfs: {} not mounted: {}", DATA_DEVICE, e),
    }

    // the boot thread (the shell) starts in its home Package
    let _ = file::chdir("/Users/root");
    Ok(())
}

pub mod devfs;
pub mod file;
pub mod initramfs;
pub mod inode;
pub mod mount;
pub mod path;
pub mod procfs;
pub mod ramfs;
pub mod vfs;
pub mod zdfs; // the old flat disk format; stays compiled until FAT32 replaces it

pub use inode::{DirEntry, FileType, Inode, Stat};

use crate::base::error::Error;
use crate::{log_info, log_warn};
use alloc::{format, string::String};

/// The standard Unix layout, parents before children: (path, permission bits).
const HIERARCHY: &[(&str, u16)] = &[
    ("/bin", 0o755),
    ("/boot", 0o755),
    ("/dev", 0o755),
    ("/etc", 0o755),
    ("/etc/zodiac", 0o755),
    ("/etc/zpm", 0o755),
    ("/home", 0o755),
    ("/lib", 0o755),
    ("/mnt", 0o755),
    ("/opt", 0o755),
    ("/package", 0o755),
    ("/package/cache", 0o755),
    ("/package/db", 0o755),
    ("/package/installed", 0o755),
    ("/package/store", 0o755),
    ("/proc", 0o555),
    ("/root", 0o700),
    ("/run", 0o755),
    ("/sys", 0o555),
    ("/tmp", 0o1777),
    ("/usr", 0o755),
    ("/usr/bin", 0o755),
    ("/usr/lib", 0o755),
    ("/usr/share", 0o755),
    ("/usr/local", 0o755),
    ("/var", 0o755),
    ("/var/cache", 0o755),
    ("/var/lib", 0o755),
    ("/var/lib/zpm", 0o755),
    ("/var/log", 0o755),
    ("/var/tmp", 0o1777),
];

/// The current user's home directory.
pub fn home_dir() -> String {
    let user = crate::user::get();
    let name = crate::user::as_str(&user);
    if name == "root" {
        String::from("/root")
    } else {
        format!("/home/{}", name)
    }
}

/// Builds the root filesystem: ramfs at "/", the standard directories, /dev and
/// /proc, and whatever the initramfs on the boot disk contains.
pub fn init() {
    let root = ramfs::new_root();
    mount::mount("/", "ramfs", "rootfs", root).expect("filesystem: cannot mount /");

    for &(path, mode) in HIERARCHY {
        match vfs::create(path, FileType::Directory, mode) {
            Ok(_) | Err(Error::AlreadyExists) => {}
            Err(e) => log_warn!("filesystem: cannot create {}: {}", path, e),
        }
    }

    let _ = mount::mount("/dev", "devfs", "devfs", devfs::new_root());
    let _ = mount::mount("/proc", "procfs", "procfs", procfs::new_root());

    match initramfs::load() {
        Ok(count) => log_info!("initramfs: {} entries unpacked", count),
        Err(e) => log_warn!("initramfs: {}", e),
    }

    // the boot thread (the shell) starts in its home directory
    let _ = file::chdir("/root");
}

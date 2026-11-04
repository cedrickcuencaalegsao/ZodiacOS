use super::vfs;
use crate::drivers::storage;
use crate::log_warn;
use alloc::{format, string::String, vec::Vec};
use errors::{Error, Result};

/// Where the archive starts on the boot disk (see the Makefile). The first sector is
/// a header: "ZINI" + the archive length as a little-endian u32. The archive follows.
const INITRAMFS_LBA: u64 = 2048;
const MAX_SIZE: usize = 8 * 1024 * 1024;
const HEADER_LEN: usize = 110; // size of a cpio "newc" entry header

pub fn load() -> Result<usize> {
    let disk = storage::get("hda").ok_or(Error::NotFound)?;

    let mut header = [0u8; 512];
    disk.read_blocks(INITRAMFS_LBA, 1, &mut header)?;
    if &header[0..4] != b"ZINI" {
        return Err(Error::NotFound); // this image has no initramfs
    }
    let len = u32::from_le_bytes([header[4], header[5], header[6], header[7]]) as usize;
    if len == 0 || len > MAX_SIZE {
        return Err(Error::InvalidArgument);
    }

    let sectors = (len + 511) / 512;
    let mut data: Vec<u8> = Vec::new();
    data.try_reserve_exact(sectors * 512)
        .map_err(|_| Error::OutOfMemory)?;
    data.resize(sectors * 512, 0);
    disk.read_blocks(INITRAMFS_LBA + 1, sectors, &mut data)?;

    unpack(&data[..len])
}

fn hex(field: &[u8]) -> Result<u64> {
    let text = core::str::from_utf8(field).map_err(|_| Error::InvalidArgument)?;
    u64::from_str_radix(text, 16).map_err(|_| Error::InvalidArgument)
}

/// Maps an archive path (Unix-style, e.g. "bin/sh") into the ZodiacFS layout
/// (Cluster/Package/[Folder/]Item). Returns None when the entry needs no
/// action (the Cluster/Package/Folder already exists from the root layout).
///
/// The hierarchy has a fixed depth, so anything nested deeper than
/// Package/Folder/Item is flattened into the Item name with '-'.
fn remap(name: &str, is_dir: bool) -> Option<String> {
    let comps: Vec<&str> = name.split('/').filter(|c| !c.is_empty()).collect();
    let (top, tail) = comps.split_first()?;
    let mut rest: Vec<&str> = tail.to_vec();

    let (cluster, package, folder): (&str, &str, Option<&str>) = match *top {
        "bin" | "sbin" => ("Applications", "Core", None),
        "lib" => ("System", "Libraries", None),
        "etc" => ("System", "Configuration", None),
        "boot" => ("System", "Kernel", Some("Boot")),
        "usr" => ("System", "Shared", None),
        "root" => ("Users", "root", None),
        "home" => {
            let (user, r) = rest.split_first()?;
            let user = *user;
            rest = r.to_vec();
            ("Users", user, None)
        }
        other => {
            rest.insert(0, other); // unknown top-level name: keep it as part of the path
            ("System", "Files", None)
        }
    };

    let mut path = format!("/{}/{}", cluster, package);
    if let Some(f) = folder {
        if is_dir || rest.is_empty() {
            return None;
        }
        path.push('/');
        path.push_str(f);
        path.push('/');
        path.push_str(&rest.join("-"));
        return Some(path);
    }
    match rest.len() {
        0 => None,
        1 => {
            path.push('/');
            path.push_str(rest[0]); // a Folder (dir entry) or an Item directly in the Package
            Some(path)
        }
        _ => {
            if is_dir {
                return None; // deeper directories are flattened into item names
            }
            path.push('/');
            path.push_str(rest[0]); // Folder
            path.push('/');
            path.push_str(&rest[1..].join("-")); // Item
            Some(path)
        }
    }
}

/// cpio "newc": a 110-byte ASCII header ("070701" + thirteen 8-digit hex fields),
/// the NUL-terminated name, then the data. Names and data are padded to 4 bytes.
fn unpack(archive: &[u8]) -> Result<usize> {
    let mut pos = 0;
    let mut count = 0;

    while pos + HEADER_LEN <= archive.len() {
        let h = &archive[pos..pos + HEADER_LEN];
        if &h[0..6] != b"070701" {
            return Err(Error::InvalidArgument);
        }
        let field = |i: usize| hex(&h[6 + i * 8..14 + i * 8]);
        let mode = field(1)? as u32;
        let filesize = field(6)? as usize;
        let namesize = field(11)? as usize;

        let name_start = pos + HEADER_LEN;
        let name_end = name_start + namesize;
        let data_start = (name_end + 3) & !3;
        let data_end = data_start + filesize;
        if namesize == 0 || data_end > archive.len() {
            return Err(Error::InvalidArgument);
        }

        let raw = core::str::from_utf8(&archive[name_start..name_end - 1])
            .map_err(|_| Error::InvalidArgument)?;
        if raw == "TRAILER!!!" {
            break;
        }

        let name = raw.trim_start_matches("./").trim_start_matches('/');
        if !name.is_empty() {
            let perm = (mode & 0o7777) as u16;
            let result = match mode & 0o170000 {
                0o040000 => match remap(name, true) {
                    Some(path) => vfs::mkdir_all(&path, perm),
                    None => Ok(()),
                },
                0o100000 => match remap(name, false) {
                    Some(path) => vfs::write_file(&path, perm, &archive[data_start..data_end]),
                    None => Ok(()),
                },
                _ => Ok(()), // symlinks and device nodes are not supported yet
            };
            // One bad entry must not stop the boot: report it and keep going.
            match result {
                Ok(()) => count += 1,
                Err(e) => log_warn!("initramfs: skipped {}: {}", name, e),
            }
        }
        pos = (data_end + 3) & !3;
    }
    Ok(count)
}

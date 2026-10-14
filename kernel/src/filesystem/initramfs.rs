use super::vfs;
use crate::base::error::{Error, Result};
use crate::drivers::storage;
use alloc::{format, string::String, vec::Vec};

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
            let path: String = format!("/{}", name);
            match mode & 0o170000 {
                0o040000 => vfs::mkdir_all(&path, (mode & 0o7777) as u16)?,
                0o100000 => vfs::write_file(
                    &path,
                    (mode & 0o7777) as u16,
                    &archive[data_start..data_end],
                )?,
                _ => {} // symlinks and device nodes are not supported yet
            }
            count += 1;
        }
        pos = (data_end + 3) & !3;
    }
    Ok(count)
}

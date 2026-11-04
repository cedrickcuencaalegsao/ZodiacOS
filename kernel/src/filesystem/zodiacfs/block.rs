use crate::base::sync::Mutex;
use crate::drivers::storage;
use alloc::{collections::BTreeMap, vec::Vec};
use errors::{Error, Result};

pub const SECTOR: usize = 512;

/// What ZodiacFS needs from a disk. `buf`/`data` lengths are multiples of 512.
pub trait BlockDevice: Send + Sync {
    fn sectors(&self) -> u64;
    fn read(&self, lba: u64, buf: &mut [u8]) -> Result<()>;
    fn write(&self, lba: u64, data: &[u8]) -> Result<()>;
}

/// A drive registered with `drivers::storage` (e.g. "hdb").
/// ASSUMPTION: the storage handle has `write_blocks(lba, count, &[u8])` mirroring
/// the `read_blocks(lba, count, &mut [u8])` used by initramfs.rs.
pub struct DriveDisk {
    name: &'static str,
    sectors: u64,
}

impl DriveDisk {
    pub fn open(name: &'static str, sectors: u64) -> Option<DriveDisk> {
        storage::get(name)?;
        Some(DriveDisk { name, sectors })
    }
}

impl BlockDevice for DriveDisk {
    fn sectors(&self) -> u64 {
        self.sectors
    }

    fn read(&self, lba: u64, buf: &mut [u8]) -> Result<()> {
        let disk = storage::get(self.name).ok_or(Error::NotFound)?;
        disk.read_blocks(lba, buf.len() / SECTOR, buf)
    }

    fn write(&self, lba: u64, data: &[u8]) -> Result<()> {
        let disk = storage::get(self.name).ok_or(Error::NotFound)?;
        disk.write_blocks(lba, data.len() / SECTOR, data)
    }
}

/// RAM-backed disk, sparse: memory is taken in 4 KiB chunks only when a
/// non-zero sector is written. Unwritten sectors read as zeros and cost
/// nothing, so a large disk size is just a limit, not an up-front allocation.
/// Running out of heap shows up as `OutOfMemory` from `write`, never a crash.
pub struct MemDisk {
    sectors: u64,
    chunks: Mutex<BTreeMap<u64, Vec<u8>>>, // chunk index -> 8 sectors
}

const CHUNK_SECTORS: u64 = 8;
const CHUNK_BYTES: usize = CHUNK_SECTORS as usize * SECTOR;

impl MemDisk {
    pub fn new(sectors: usize) -> Result<MemDisk> {
        Ok(MemDisk {
            sectors: sectors as u64,
            chunks: Mutex::new(BTreeMap::new()),
        })
    }

    fn check(&self, lba: u64, len: usize) -> Result<()> {
        let count = (len / SECTOR) as u64;
        if len % SECTOR != 0
            || lba
                .checked_add(count)
                .map_or(true, |end| end > self.sectors)
        {
            return Err(Error::InvalidArgument);
        }
        Ok(())
    }
}

impl BlockDevice for MemDisk {
    fn sectors(&self) -> u64 {
        self.sectors
    }

    fn read(&self, lba: u64, buf: &mut [u8]) -> Result<()> {
        self.check(lba, buf.len())?;
        let chunks = self.chunks.lock();
        for (i, out) in buf.chunks_mut(SECTOR).enumerate() {
            let s = lba + i as u64;
            let off = (s % CHUNK_SECTORS) as usize * SECTOR;
            match chunks.get(&(s / CHUNK_SECTORS)) {
                Some(chunk) => out.copy_from_slice(&chunk[off..off + SECTOR]),
                None => out.fill(0),
            }
        }
        Ok(())
    }

    fn write(&self, lba: u64, data: &[u8]) -> Result<()> {
        self.check(lba, data.len())?;
        let mut chunks = self.chunks.lock();
        for (i, src) in data.chunks(SECTOR).enumerate() {
            let s = lba + i as u64;
            let key = s / CHUNK_SECTORS;
            let off = (s % CHUNK_SECTORS) as usize * SECTOR;
            if !chunks.contains_key(&key) {
                if src.iter().all(|&b| b == 0) {
                    continue; // zeros need no memory
                }
                let mut chunk: Vec<u8> = Vec::new();
                chunk
                    .try_reserve_exact(CHUNK_BYTES)
                    .map_err(|_| Error::OutOfMemory)?;
                chunk.resize(CHUNK_BYTES, 0);
                chunks.insert(key, chunk);
            }
            if let Some(chunk) = chunks.get_mut(&key) {
                chunk[off..off + SECTOR].copy_from_slice(src);
            }
        }
        Ok(())
    }
}

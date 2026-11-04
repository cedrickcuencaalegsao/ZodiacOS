//! ZodiacFS binary disk format (all integers little-endian).
//!
//!   sector 0                 Container header (checksummed)
//!   sectors 1..=1024         Object table: 4096 records x 128 bytes (checksummed each)
//!   sector 1032 (block 0)..  Data region, allocated in 4 KiB blocks (8 sectors)
//!
//! One record describes one Cluster, Package, Folder or Item. A record fits in
//! a quarter of a sector, so updating it is a single atomic sector write.

use super::block::SECTOR;
use super::integrity::crc32;
use alloc::string::String;
use errors::{Error, Result};

pub const BLOCK_SECTORS: u64 = 8;
pub const BLOCK_SIZE: usize = 4096;
pub const RECORD_SIZE: usize = 128;
pub const RECORDS_PER_SECTOR: usize = SECTOR / RECORD_SIZE;
pub const TABLE_START: u64 = 1;
pub const TABLE_SECTORS: u64 = 1024;
pub const MAX_RECORDS: usize = TABLE_SECTORS as usize * RECORDS_PER_SECTOR;
pub const DATA_START: u64 = 1032; // 1 + 1024, rounded up to a block boundary
pub const MAX_NAME: usize = 44;
pub const MAX_LABEL: usize = 32;
pub const MAX_EXTENTS: usize = 4;
pub const FIRST_ID: u64 = 2; // id 0 = container root (never stored); 1 reserved

const MAGIC: [u8; 8] = *b"ZODIACFS";
pub const VERSION: u32 = 1;

/// Single place to change if `errors::Error` gains a `Corrupted` variant.
pub fn corrupt() -> Error {
    Error::InvalidArgument
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Root, // the Container itself, in memory only
    Cluster,
    Package,
    Folder,
    Item,
}

impl Kind {
    fn to_u8(self) -> u8 {
        match self {
            Kind::Root => 0,
            Kind::Cluster => 1,
            Kind::Package => 2,
            Kind::Folder => 3,
            Kind::Item => 4,
        }
    }
    fn from_u8(v: u8) -> Option<Kind> {
        Some(match v {
            1 => Kind::Cluster,
            2 => Kind::Package,
            3 => Kind::Folder,
            4 => Kind::Item,
            _ => return None,
        })
    }
}

/// The fixed hierarchy (FILESYSTEM.md §2): Container > Cluster > Package > Folder > Item,
/// with Items also allowed directly inside a Package. Nothing else nests.
pub fn allowed(parent: Kind, child: Kind) -> bool {
    matches!(
        (parent, child),
        (Kind::Root, Kind::Cluster)
            | (Kind::Cluster, Kind::Package)
            | (Kind::Package, Kind::Folder)
            | (Kind::Package, Kind::Item)
            | (Kind::Folder, Kind::Item)
    )
}

#[derive(Clone, Copy, Default)]
pub struct Extent {
    pub start: u32, // first block
    pub len: u32,   // 0 = unused
}

fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}
fn u64_at(b: &[u8], o: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[o..o + 8]);
    u64::from_le_bytes(a)
}

/// Object record (128 bytes):
///   0 kind | 1 name_len | 2..4 mode | 4..8 uid | 8..12 gid | 12..16 crc32
///   16..24 id | 24..32 parent id | 32..40 size | 40..44 ctime | 44..48 mtime
///   48..80 four extents (start u32, len u32) | 80..124 name | 124..128 reserved
#[derive(Clone)]
pub struct Record {
    pub kind: Kind,
    pub mode: u16,
    pub uid: u32,
    pub gid: u32,
    pub id: u64,
    pub parent: u64,
    pub size: u64,
    pub ctime: u32,
    pub mtime: u32,
    pub extents: [Extent; MAX_EXTENTS],
    pub name: String,
}

impl Record {
    pub fn encode(&self) -> [u8; RECORD_SIZE] {
        let mut b = [0u8; RECORD_SIZE];
        b[0] = self.kind.to_u8();
        b[1] = self.name.len() as u8;
        b[2..4].copy_from_slice(&self.mode.to_le_bytes());
        b[4..8].copy_from_slice(&self.uid.to_le_bytes());
        b[8..12].copy_from_slice(&self.gid.to_le_bytes());
        b[16..24].copy_from_slice(&self.id.to_le_bytes());
        b[24..32].copy_from_slice(&self.parent.to_le_bytes());
        b[32..40].copy_from_slice(&self.size.to_le_bytes());
        b[40..44].copy_from_slice(&self.ctime.to_le_bytes());
        b[44..48].copy_from_slice(&self.mtime.to_le_bytes());
        for (i, e) in self.extents.iter().enumerate() {
            b[48 + i * 8..52 + i * 8].copy_from_slice(&e.start.to_le_bytes());
            b[52 + i * 8..56 + i * 8].copy_from_slice(&e.len.to_le_bytes());
        }
        b[80..80 + self.name.len()].copy_from_slice(self.name.as_bytes());
        let crc = crc32(&b); // crc field is still zero here
        b[12..16].copy_from_slice(&crc.to_le_bytes());
        b
    }

    /// `Ok(None)` for a free slot, `Err` for a damaged record.
    pub fn decode(raw: &[u8]) -> Result<Option<Record>> {
        let mut b = [0u8; RECORD_SIZE];
        b.copy_from_slice(&raw[..RECORD_SIZE]);
        if b.iter().all(|&x| x == 0) {
            return Ok(None);
        }
        let stored = u32_at(&b, 12);
        b[12..16].fill(0);
        if crc32(&b) != stored {
            return Err(corrupt());
        }
        let kind = Kind::from_u8(b[0]).ok_or_else(corrupt)?;
        let name_len = b[1] as usize;
        if name_len == 0 || name_len > MAX_NAME {
            return Err(corrupt());
        }
        let name = core::str::from_utf8(&b[80..80 + name_len]).map_err(|_| corrupt())?;
        let mut extents = [Extent::default(); MAX_EXTENTS];
        for (i, e) in extents.iter_mut().enumerate() {
            e.start = u32_at(&b, 48 + i * 8);
            e.len = u32_at(&b, 52 + i * 8);
        }
        Ok(Some(Record {
            kind,
            mode: u16_at(&b, 2),
            uid: u32_at(&b, 4),
            gid: u32_at(&b, 8),
            id: u64_at(&b, 16),
            parent: u64_at(&b, 24),
            size: u64_at(&b, 32),
            ctime: u32_at(&b, 40),
            mtime: u32_at(&b, 44),
            extents,
            name: String::from(name),
        }))
    }
}

/// Container header (sector 0):
///   0..8 magic | 8..12 version | 12..16 block size | 16..24 container id
///   24..32 total sectors | 32..40 next object id | 40..72 label | 72 label_len
///   508..512 crc32 of bytes 0..508
pub struct Header {
    pub id: u64,
    pub label: String,
    pub total_sectors: u64,
    pub next_id: u64,
}

impl Header {
    pub fn encode(&self) -> [u8; SECTOR] {
        let mut b = [0u8; SECTOR];
        b[0..8].copy_from_slice(&MAGIC);
        b[8..12].copy_from_slice(&VERSION.to_le_bytes());
        b[12..16].copy_from_slice(&(BLOCK_SIZE as u32).to_le_bytes());
        b[16..24].copy_from_slice(&self.id.to_le_bytes());
        b[24..32].copy_from_slice(&self.total_sectors.to_le_bytes());
        b[32..40].copy_from_slice(&self.next_id.to_le_bytes());
        b[40..40 + self.label.len()].copy_from_slice(self.label.as_bytes());
        b[72] = self.label.len() as u8;
        let crc = crc32(&b[..508]);
        b[508..512].copy_from_slice(&crc.to_le_bytes());
        b
    }

    pub fn decode(b: &[u8; SECTOR]) -> Result<Header> {
        if b[0..8] != MAGIC {
            return Err(Error::NotFound); // not a ZodiacFS container
        }
        if crc32(&b[..508]) != u32_at(b, 508)
            || u32_at(b, 8) != VERSION
            || u32_at(b, 12) as usize != BLOCK_SIZE
        {
            return Err(corrupt());
        }
        let n = b[72] as usize;
        if n > MAX_LABEL {
            return Err(corrupt());
        }
        let label = core::str::from_utf8(&b[40..40 + n]).map_err(|_| corrupt())?;
        let total_sectors = u64_at(b, 24);
        if total_sectors < DATA_START + BLOCK_SECTORS {
            return Err(corrupt());
        }
        Ok(Header {
            id: u64_at(b, 16),
            label: String::from(label),
            total_sectors,
            next_id: u64_at(b, 32).max(FIRST_ID),
        })
    }
}

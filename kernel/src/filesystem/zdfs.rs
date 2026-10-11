use crate::{ata, time, user};

pub const DRIVE: u8 = 1; // hdb = the data disk

const MAGIC: u32 = 0x5346_445A; // "ZDFS"
const TABLE_START: u32 = 1;
const TABLE_SECTORS: u32 = 16;
const DATA_START: u32 = TABLE_START + TABLE_SECTORS;
const ENTRY_SIZE: usize = 64;
const PER_SECTOR: usize = 512 / ENTRY_SIZE;
const SLOTS: usize = TABLE_SECTORS as usize * PER_SECTOR;
pub const MAX_NAME: usize = 20;

// Entry layout (64 bytes):
//   0..20 name | 20..22 parent id | 22 used | 23 kind (0 file, 1 dir)
//   24..28 start sector | 28..32 size in bytes
//   32..40 author | 40..44 created (unix) | 44..48 modified (unix)

/// Folder identifier: 0 is the root, otherwise (table slot + 1).
pub type Id = u16;
pub const ROOT: Id = 0;

type R<T> = Result<T, &'static str>;

#[derive(Clone, Copy)]
pub struct Entry {
    name: [u8; MAX_NAME],
    author: [u8; user::MAX],
    pub parent: Id,
    pub is_dir: bool,
    pub start: u32,
    pub size: u32,
    pub created: u32,
    pub modified: u32,
    slot: usize,
}

impl Entry {
    /// A fresh entry authored by the current user, stamped with the current time.
    fn new(name: &str, parent: Id, is_dir: bool, slot: usize) -> Entry {
        let mut n = [0u8; MAX_NAME];
        n[..name.len()].copy_from_slice(name.as_bytes());
        let now = time::now();
        Entry {
            name: n,
            author: user::get(),
            parent,
            is_dir,
            start: 0,
            size: 0,
            created: now,
            modified: now,
            slot,
        }
    }

    pub fn id(&self) -> Id {
        self.slot as Id + 1
    }

    pub fn name(&self) -> &str {
        user::as_str(&self.name)
    }

    pub fn author(&self) -> &str {
        user::as_str(&self.author)
    }
}

// ---- Low-level helpers ------------------------------------------------------
fn read_sector(lba: u32, buf: &mut [u8; 512]) -> R<()> {
    if ata::read_sector(DRIVE, lba, buf) { Ok(()) } else { Err("disk read error") }
}

fn write_sector(lba: u32, buf: &[u8; 512]) -> R<()> {
    if ata::write_sector(DRIVE, lba, buf) { Ok(()) } else { Err("disk write error") }
}

fn total_sectors() -> R<u32> {
    Ok(ata::identify(DRIVE).ok_or("no data disk (hdb)")?.sectors)
}

/// Checks the disk is formatted and returns the next free data sector.
fn superblock() -> R<u32> {
    total_sectors()?;
    let mut b = [0u8; 512];
    read_sector(0, &mut b)?;
    if u32::from_le_bytes([b[0], b[1], b[2], b[3]]) != MAGIC {
        return Err("disk not formatted (run 'format')");
    }
    Ok(u32::from_le_bytes([b[4], b[5], b[6], b[7]]))
}

fn set_next_free(v: u32) -> R<()> {
    let mut b = [0u8; 512];
    b[0..4].copy_from_slice(&MAGIC.to_le_bytes());
    b[4..8].copy_from_slice(&v.to_le_bytes());
    write_sector(0, &b)
}

fn parse(buf: &[u8; 512], table_sector: usize, idx: usize) -> Option<Entry> {
    let off = idx * ENTRY_SIZE;
    if buf[off + 22] == 0 {
        return None; // unused slot
    }
    let mut name = [0u8; MAX_NAME];
    name.copy_from_slice(&buf[off..off + MAX_NAME]);
    let mut author = [0u8; user::MAX];
    author.copy_from_slice(&buf[off + 32..off + 32 + user::MAX]);
    Some(Entry {
        name,
        author,
        parent: u16::from_le_bytes([buf[off + 20], buf[off + 21]]),
        is_dir: buf[off + 23] == 1,
        start: u32::from_le_bytes(buf[off + 24..off + 28].try_into().unwrap()),
        size: u32::from_le_bytes(buf[off + 28..off + 32].try_into().unwrap()),
        created: u32::from_le_bytes(buf[off + 40..off + 44].try_into().unwrap()),
        modified: u32::from_le_bytes(buf[off + 44..off + 48].try_into().unwrap()),
        slot: table_sector * PER_SECTOR + idx,
    })
}

fn store_entry(slot: usize, e: Option<&Entry>) -> R<()> {
    let lba = TABLE_START + (slot / PER_SECTOR) as u32;
    let off = (slot % PER_SECTOR) * ENTRY_SIZE;
    let mut b = [0u8; 512];
    read_sector(lba, &mut b)?;
    b[off..off + ENTRY_SIZE].fill(0);
    if let Some(e) = e {
        b[off..off + MAX_NAME].copy_from_slice(&e.name);
        b[off + 20..off + 22].copy_from_slice(&e.parent.to_le_bytes());
        b[off + 22] = 1; // used flag
        b[off + 23] = e.is_dir as u8;
        b[off + 24..off + 28].copy_from_slice(&e.start.to_le_bytes());
        b[off + 28..off + 32].copy_from_slice(&e.size.to_le_bytes());
        b[off + 32..off + 32 + user::MAX].copy_from_slice(&e.author);
        b[off + 40..off + 44].copy_from_slice(&e.created.to_le_bytes());
        b[off + 44..off + 48].copy_from_slice(&e.modified.to_le_bytes());
    }
    write_sector(lba, &b)
}

/// Calls `f` for every used entry; stop early by returning true.
fn scan(mut f: impl FnMut(Entry) -> bool) -> R<()> {
    let mut b = [0u8; 512];
    for s in 0..TABLE_SECTORS as usize {
        read_sector(TABLE_START + s as u32, &mut b)?;
        for i in 0..PER_SECTOR {
            if let Some(e) = parse(&b, s, i) {
                if f(e) {
                    return Ok(());
                }
            }
        }
    }
    Ok(())
}

fn find(name: &str, parent: Id) -> R<Option<Entry>> {
    let mut found = None;
    scan(|e| {
        if e.parent == parent && e.name() == name {
            found = Some(e);
            true
        } else {
            false
        }
    })?;
    Ok(found)
}

fn find_free_slot() -> R<usize> {
    let mut b = [0u8; 512];
    for s in 0..TABLE_SECTORS as usize {
        read_sector(TABLE_START + s as u32, &mut b)?;
        for i in 0..PER_SECTOR {
            if b[i * ENTRY_SIZE + 22] == 0 {
                return Ok(s * PER_SECTOR + i);
            }
        }
    }
    Err("file table full")
}

fn check_name(name: &str) -> R<()> {
    if name.is_empty()
        || name.len() > MAX_NAME
        || name == "."
        || name == ".."
        || !name.bytes().all(|b| b > 0x20 && b < 0x7F && b != b'/')
    {
        return Err("invalid name (1-20 printable characters, no '/')");
    }
    Ok(())
}

fn create(name: &str, parent: Id, is_dir: bool) -> R<()> {
    check_name(name)?;
    superblock()?;
    if find(name, parent)?.is_some() {
        return Err("already exists");
    }
    let slot = find_free_slot()?;
    store_entry(slot, Some(&Entry::new(name, parent, is_dir, slot)))
}

// ---- Public API -------------------------------------------------------------
pub fn format() -> R<()> {
    total_sectors()?;
    set_next_free(DATA_START)?;
    let zero = [0u8; 512];
    for i in 0..TABLE_SECTORS {
        write_sector(TABLE_START + i, &zero)?;
    }
    Ok(())
}

/// Creates an empty file in `parent`.
pub fn create_file(name: &str, parent: Id) -> R<()> {
    create(name, parent, false)
}

/// Creates an empty folder in `parent`.
pub fn mkdir(name: &str, parent: Id) -> R<()> {
    create(name, parent, true)
}

/// Looks up an entry (file or folder) by name inside `parent`.
pub fn lookup(name: &str, parent: Id) -> R<Entry> {
    superblock()?;
    find(name, parent)?.ok_or("not found")
}

/// Fetches an entry by its id (used to walk up to a parent folder).
pub fn get(id: Id) -> R<Entry> {
    if id == ROOT || id as usize > SLOTS {
        return Err("bad id");
    }
    let slot = id as usize - 1;
    let mut b = [0u8; 512];
    read_sector(TABLE_START + (slot / PER_SECTOR) as u32, &mut b)?;
    parse(&b, slot / PER_SECTOR, slot % PER_SECTOR).ok_or("not found")
}

pub fn list(parent: Id, mut f: impl FnMut(&Entry)) -> R<()> {
    superblock()?;
    scan(|e| {
        if e.parent == parent {
            f(&e);
        }
        false
    })
}

pub fn read(name: &str, parent: Id, mut f: impl FnMut(&[u8])) -> R<()> {
    superblock()?;
    let e = find(name, parent)?.ok_or("file not found")?;
    if e.is_dir {
        return Err("is a directory");
    }
    let mut remaining = e.size as usize;
    let mut lba = e.start;
    let mut buf = [0u8; 512];
    while remaining > 0 {
        read_sector(lba, &mut buf)?;
        let n = remaining.min(512);
        f(&buf[..n]);
        remaining -= n;
        lba += 1;
    }
    Ok(())
}

pub fn write(name: &str, parent: Id, data: &[u8]) -> R<()> {
    check_name(name)?;
    let next_free = superblock()?;
    let total = total_sectors()?;
    let need = ((data.len() + 511) / 512) as u32;
    let existing = find(name, parent)?;

    if let Some(e) = &existing {
        if e.is_dir {
            return Err("is a directory");
        }
    }
    let slot = match &existing {
        Some(e) => e.slot,
        None => find_free_slot()?,
    };

    // Reuse the old space if the new data fits; otherwise allocate at the end.
    let (start, new_next) = match &existing {
        Some(old) if need <= (old.size + 511) / 512 => (old.start, next_free),
        _ => {
            if next_free + need > total {
                return Err("disk full");
            }
            (next_free, next_free + need)
        }
    };

    for i in 0..need as usize {
        let mut buf = [0u8; 512];
        let chunk = &data[i * 512..data.len().min((i + 1) * 512)];
        buf[..chunk.len()].copy_from_slice(chunk);
        write_sector(start + i as u32, &buf)?;
    }

    // Overwriting keeps the original author and creation time, and bumps `modified`.
    let mut entry = match existing {
        Some(e) => e,
        None => Entry::new(name, parent, false, slot),
    };
    entry.start = start;
    entry.size = data.len() as u32;
    entry.modified = time::now();
    store_entry(slot, Some(&entry))?;

    if new_next != next_free {
        set_next_free(new_next)?;
    }
    Ok(())
}

/// Removes a file or an empty folder.
pub fn remove(name: &str, parent: Id) -> R<()> {
    superblock()?;
    let e = find(name, parent)?.ok_or("not found")?;
    if e.is_dir {
        let mut has_children = false;
        scan(|c| {
            if c.parent == e.id() {
                has_children = true;
                true
            } else {
                false
            }
        })?;
        if has_children {
            return Err("folder is not empty");
        }
    }
    store_entry(e.slot, None)
}

/// (used sectors, total sectors)
pub fn usage() -> R<(u32, u32)> {
    let used = superblock()?;
    Ok((used, total_sectors()?))
}
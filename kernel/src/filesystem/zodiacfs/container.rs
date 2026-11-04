//! A mounted ZodiacFS Container: object table, object index, name index,
//! allocation map, and the Item data path.
//!
//! Crash-consistency rule used everywhere: write data first, then commit by
//! rewriting the one 512-byte sector holding the object record, and only then
//! release old blocks. A crash leaves either the old or the new state.

use super::super::inode::FileType;
use super::super::inode::Stat;
use super::super::path::check_name;
use super::allocator::Allocator;
use super::block::{BlockDevice, SECTOR};
use super::layout::*;
use crate::time;
use alloc::{boxed::Box, collections::BTreeMap, string::String, vec::Vec};
use errors::{Error, Result};

pub const ROOT: u64 = 0;

struct Obj {
    rec: Record,
    slot: u32,
}

pub struct Container {
    dev: Box<dyn BlockDevice>,
    hdr: Header,
    objs: BTreeMap<u64, Obj>, // Object Index: id -> record (+ table slot)
    names: BTreeMap<(u64, String), u64>, // Name Index: (parent id, name) -> id
    used_slots: Vec<bool>,
    alloc: Allocator,
}

fn blocks_for(size: u64) -> Result<u32> {
    let blocks = size / BLOCK_SIZE as u64 + (size % BLOCK_SIZE as u64 != 0) as u64;
    u32::try_from(blocks).map_err(|_| Error::NoSpace)
}

fn sector_in(extents: &[Extent; MAX_EXTENTS], blk: u32) -> Result<u64> {
    let mut rem = blk;
    for e in extents {
        if rem < e.len {
            return Ok(DATA_START + (e.start + rem) as u64 * BLOCK_SECTORS);
        }
        rem -= e.len;
    }
    Err(corrupt())
}

/// Initializes an empty Container on `dev`. Destructive; the caller must have
/// obtained user authorization. The header is written last, so an interrupted
/// format leaves the disk "unformatted" rather than half-valid.
pub fn format(dev: &dyn BlockDevice, label: &str, container_id: u64) -> Result<()> {
    if label.is_empty() || label.len() > MAX_LABEL {
        return Err(Error::InvalidArgument);
    }
    if dev.sectors() < DATA_START + BLOCK_SECTORS * 16 {
        return Err(Error::NoSpace);
    }
    let zero = [0u8; BLOCK_SIZE];
    let mut s = 0;
    while s < TABLE_SECTORS {
        dev.write(TABLE_START + s, &zero)?;
        s += BLOCK_SECTORS;
    }
    let hdr = Header {
        id: container_id,
        label: String::from(label),
        total_sectors: dev.sectors(),
        next_id: FIRST_ID,
    };
    dev.write(0, &hdr.encode())
}

impl Container {
    /// Reads, validates and indexes a Container. Rejects anything inconsistent.
    pub fn mount(dev: Box<dyn BlockDevice>) -> Result<Container> {
        let mut sec = [0u8; SECTOR];
        dev.read(0, &mut sec)?;
        let hdr = Header::decode(&sec)?;
        if hdr.total_sectors > dev.sectors() {
            return Err(corrupt());
        }
        let total_blocks = ((hdr.total_sectors - DATA_START) / BLOCK_SECTORS) as u32;
        let mut c = Container {
            dev,
            hdr,
            objs: BTreeMap::new(),
            names: BTreeMap::new(),
            used_slots: alloc::vec![false; MAX_RECORDS],
            alloc: Allocator::new(total_blocks),
        };

        // 1. read every record
        let mut tbl = [0u8; BLOCK_SIZE];
        let mut s = 0u64;
        while s < TABLE_SECTORS {
            c.dev.read(TABLE_START + s, &mut tbl)?;
            for i in 0..(BLOCK_SECTORS as usize * RECORDS_PER_SECTOR) {
                let slot = s as usize * RECORDS_PER_SECTOR + i;
                if let Some(rec) = Record::decode(&tbl[i * RECORD_SIZE..(i + 1) * RECORD_SIZE])? {
                    c.used_slots[slot] = true;
                    if c.objs
                        .insert(
                            rec.id,
                            Obj {
                                rec,
                                slot: slot as u32,
                            },
                        )
                        .is_some()
                    {
                        return Err(corrupt()); // duplicate object id
                    }
                }
            }
            s += BLOCK_SECTORS;
        }

        // 2. validate the hierarchy, build the name index and allocation map
        let ids: Vec<u64> = c.objs.keys().copied().collect();
        for id in ids {
            let (parent, kind, name, extents) = {
                let r = &c.objs[&id].rec;
                (r.parent, r.kind, r.name.clone(), r.extents)
            };
            let pk = c.kind_of(parent).ok_or_else(corrupt)?;
            if id < FIRST_ID || !allowed(pk, kind) {
                return Err(corrupt());
            }
            if c.names.insert((parent, name), id).is_some() {
                return Err(corrupt()); // duplicate name in one parent
            }
            for e in extents.iter().filter(|e| e.len > 0) {
                if kind != Kind::Item {
                    return Err(corrupt());
                }
                c.alloc.mark(e.start, e.len).map_err(|_| corrupt())?;
            }
            if id >= c.hdr.next_id {
                c.hdr.next_id = id + 1;
            }
        }
        Ok(c)
    }

    pub fn label(&self) -> &str {
        &self.hdr.label
    }

    /// (used bytes, total bytes) of the data region.
    pub fn usage(&self) -> (u64, u64) {
        let (used, total) = self.alloc.counts();
        (
            used as u64 * BLOCK_SIZE as u64,
            total as u64 * BLOCK_SIZE as u64,
        )
    }

    pub fn kind_of(&self, id: u64) -> Option<Kind> {
        if id == ROOT {
            Some(Kind::Root)
        } else {
            self.objs.get(&id).map(|o| o.rec.kind)
        }
    }

    fn kids(&self, parent: u64) -> impl Iterator<Item = (&String, u64)> + '_ {
        self.names
            .range((parent, String::new())..)
            .take_while(move |(k, _)| k.0 == parent)
            .map(|(k, id)| (&k.1, *id))
    }

    // ---- low-level persistence ------------------------------------------------

    fn write_header(&self) -> Result<()> {
        self.dev.write(0, &self.hdr.encode())
    }

    fn store(&self, slot: u32, rec: Option<&Record>) -> Result<()> {
        let lba = TABLE_START + (slot as usize / RECORDS_PER_SECTOR) as u64;
        let off = (slot as usize % RECORDS_PER_SECTOR) * RECORD_SIZE;
        let mut b = [0u8; SECTOR];
        self.dev.read(lba, &mut b)?;
        match rec {
            Some(r) => b[off..off + RECORD_SIZE].copy_from_slice(&r.encode()),
            None => b[off..off + RECORD_SIZE].fill(0),
        }
        self.dev.write(lba, &b)
    }

    fn commit(&mut self, rec: Record) -> Result<()> {
        let slot = self.objs.get(&rec.id).ok_or(Error::NotFound)?.slot;
        self.store(slot, Some(&rec))?;
        if let Some(o) = self.objs.get_mut(&rec.id) {
            o.rec = rec;
        }
        Ok(())
    }

    fn release(&mut self, extents: &[Extent; MAX_EXTENTS]) {
        for e in extents.iter().filter(|e| e.len > 0) {
            self.alloc.free(e.start, e.len);
        }
    }

    // ---- namespace -----------------------------------------------------------

    pub fn lookup(&self, parent: u64, name: &str) -> Result<u64> {
        self.names
            .get(&(parent, String::from(name)))
            .copied()
            .ok_or(Error::NotFound)
    }

    pub fn create(&mut self, parent: u64, name: &str, kind: Kind, mode: u16) -> Result<u64> {
        check_name(name)?;
        if name.len() > MAX_NAME {
            return Err(Error::NameTooLong);
        }
        let pk = self.kind_of(parent).ok_or(Error::NotFound)?;
        if pk == Kind::Item {
            return Err(Error::NotADirectory);
        }
        if !allowed(pk, kind) {
            return Err(Error::InvalidArgument); // fixed-depth hierarchy
        }
        let key = (parent, String::from(name));
        if self.names.contains_key(&key) {
            return Err(Error::AlreadyExists);
        }
        let slot = self
            .used_slots
            .iter()
            .position(|u| !*u)
            .ok_or(Error::NoSpace)?;
        let id = self.hdr.next_id;
        let now = time::now();
        let rec = Record {
            kind,
            mode,
            uid: 0,
            gid: 0,
            id,
            parent,
            size: 0,
            ctime: now,
            mtime: now,
            extents: [Extent::default(); MAX_EXTENTS],
            name: String::from(name),
        };
        // Persist the id counter first so an id is never reused after a crash.
        self.hdr.next_id += 1;
        self.write_header()?;
        self.store(slot as u32, Some(&rec))?;
        self.used_slots[slot] = true;
        self.names.insert(key, id);
        self.objs.insert(
            id,
            Obj {
                rec,
                slot: slot as u32,
            },
        );
        Ok(id)
    }

    pub fn remove(&mut self, parent: u64, name: &str) -> Result<()> {
        let id = self.lookup(parent, name)?;
        let (kind, extents, slot) = {
            let o = &self.objs[&id];
            (o.rec.kind, o.rec.extents, o.slot)
        };
        if kind != Kind::Item && self.kids(id).next().is_some() {
            return Err(Error::NotEmpty);
        }
        self.store(slot, None)?; // commit
        self.release(&extents);
        self.names.remove(&(parent, String::from(name)));
        self.objs.remove(&id);
        self.used_slots[slot as usize] = false;
        Ok(())
    }

    pub fn read_dir(&self, parent: u64) -> Result<Vec<(String, Kind, u64)>> {
        if self.kind_of(parent).ok_or(Error::NotFound)? == Kind::Item {
            return Err(Error::NotADirectory);
        }
        Ok(self
            .kids(parent)
            .map(|(n, id)| (n.clone(), self.objs[&id].rec.kind, id))
            .collect())
    }

    pub fn stat(&self, id: u64) -> Result<Stat> {
        if id == ROOT {
            return Ok(Stat {
                ino: 1,
                kind: FileType::Directory,
                size: self.kids(ROOT).count() as u64,
                mode: 0o755,
                uid: 0,
                gid: 0,
                mtime: 0,
                nlink: 2,
            });
        }
        let r = &self.objs.get(&id).ok_or(Error::NotFound)?.rec;
        let is_item = r.kind == Kind::Item;
        Ok(Stat {
            ino: r.id,
            kind: if is_item {
                FileType::File
            } else {
                FileType::Directory
            },
            size: if is_item {
                r.size
            } else {
                self.kids(id).count() as u64
            },
            mode: r.mode,
            uid: r.uid,
            gid: r.gid,
            mtime: r.mtime,
            nlink: if is_item { 1 } else { 2 },
        })
    }

    // ---- Item data -----------------------------------------------------------

    fn item(&self, id: u64) -> Result<&Record> {
        let r = &self.objs.get(&id).ok_or(Error::NotFound)?.rec;
        if r.kind != Kind::Item {
            return Err(Error::IsADirectory);
        }
        Ok(r)
    }

    pub fn read(&self, id: u64, offset: u64, buf: &mut [u8]) -> Result<usize> {
        let rec = self.item(id)?;
        if offset >= rec.size {
            return Ok(0);
        }
        let n = buf.len().min((rec.size - offset) as usize);
        let mut tmp = [0u8; BLOCK_SIZE];
        let mut done = 0;
        while done < n {
            let pos = offset + done as u64;
            let inb = (pos % BLOCK_SIZE as u64) as usize;
            let chunk = (n - done).min(BLOCK_SIZE - inb);
            let sec = sector_in(&rec.extents, (pos / BLOCK_SIZE as u64) as u32)?;
            self.dev.read(sec, &mut tmp)?;
            buf[done..done + chunk].copy_from_slice(&tmp[inb..inb + chunk]);
            done += chunk;
        }
        Ok(n)
    }

    /// Moves to a larger allocation. New blocks are zeroed, so a hole or a
    /// recycled block can never expose another file's old data.
    fn grow(&mut self, rec: &mut Record, need: u32) -> Result<()> {
        let have: u32 = rec.extents.iter().map(|e| e.len).sum();
        if have >= need {
            return Ok(());
        }
        // over-allocate by 50% so appends don't copy the file every time
        let want = need.max(have.saturating_add(have / 2));
        let fresh = match self.alloc.alloc(want) {
            Some(f) => f,
            None => self.alloc.alloc(need).ok_or(Error::NoSpace)?,
        };
        let new_total: u32 = fresh.iter().map(|e| e.len).sum();
        let old = rec.extents;
        if let Err(e) = self.migrate(&old, &fresh, have, new_total) {
            self.release(&fresh);
            return Err(e);
        }
        let mut updated = rec.clone();
        updated.extents = fresh;
        if let Err(e) = self.commit(updated) {
            self.release(&fresh);
            return Err(e);
        }
        rec.extents = fresh;
        self.release(&old);
        Ok(())
    }

    fn migrate(
        &self,
        old: &[Extent; MAX_EXTENTS],
        fresh: &[Extent; MAX_EXTENTS],
        have: u32,
        total: u32,
    ) -> Result<()> {
        let mut tmp = [0u8; BLOCK_SIZE];
        for b in 0..have {
            self.dev.read(sector_in(old, b)?, &mut tmp)?;
            self.dev.write(sector_in(fresh, b)?, &tmp)?;
        }
        tmp.fill(0);
        for b in have..total {
            self.dev.write(sector_in(fresh, b)?, &tmp)?;
        }
        Ok(())
    }

    pub fn write(&mut self, id: u64, offset: u64, data: &[u8]) -> Result<usize> {
        if data.is_empty() {
            return Ok(0);
        }
        let end = offset
            .checked_add(data.len() as u64)
            .ok_or(Error::InvalidArgument)?;
        let mut rec = self.item(id)?.clone();
        self.grow(&mut rec, blocks_for(end)?)?;

        let mut tmp = [0u8; BLOCK_SIZE];
        let mut done = 0;
        while done < data.len() {
            let pos = offset + done as u64;
            let inb = (pos % BLOCK_SIZE as u64) as usize;
            let chunk = (data.len() - done).min(BLOCK_SIZE - inb);
            let sec = sector_in(&rec.extents, (pos / BLOCK_SIZE as u64) as u32)?;
            if chunk < BLOCK_SIZE {
                self.dev.read(sec, &mut tmp)?;
            }
            tmp[inb..inb + chunk].copy_from_slice(&data[done..done + chunk]);
            self.dev.write(sec, &tmp)?;
            done += chunk;
        }
        if end > rec.size {
            rec.size = end;
        }
        rec.mtime = time::now();
        self.commit(rec)?;
        Ok(data.len())
    }

    pub fn truncate(&mut self, id: u64, size: u64) -> Result<()> {
        let mut rec = self.item(id)?.clone();
        if size == rec.size {
            return Ok(());
        }
        if size > rec.size {
            self.grow(&mut rec, blocks_for(size)?)?;
        } else if size > 0 {
            self.zero_from(&rec, size)?; // later growth must read zeros, not old data
        }
        rec.mtime = time::now();
        if size == 0 {
            let old = rec.extents;
            rec.extents = [Extent::default(); MAX_EXTENTS];
            rec.size = 0;
            self.commit(rec)?;
            self.release(&old);
        } else {
            rec.size = size;
            self.commit(rec)?;
        }
        Ok(())
    }

    /// Zeroes everything at or after byte `size` inside the allocated blocks.
    fn zero_from(&self, rec: &Record, size: u64) -> Result<()> {
        let have: u32 = rec.extents.iter().map(|e| e.len).sum();
        let mut blk = (size / BLOCK_SIZE as u64) as u32;
        let inb = (size % BLOCK_SIZE as u64) as usize;
        let mut tmp = [0u8; BLOCK_SIZE];
        if inb != 0 && blk < have {
            let sec = sector_in(&rec.extents, blk)?;
            self.dev.read(sec, &mut tmp)?;
            tmp[inb..].fill(0);
            self.dev.write(sec, &tmp)?;
            blk += 1;
        }
        tmp.fill(0);
        while blk < have {
            self.dev.write(sector_in(&rec.extents, blk)?, &tmp)?;
            blk += 1;
        }
        Ok(())
    }
}

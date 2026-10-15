use super::inode::{FileType, Inode};
use super::mount;
use super::path::split_parent;
use crate::base::error::{Error, Result};
use alloc::{string::String, sync::Arc};

/// Finds the inode for a canonical absolute path, crossing mount points.
pub fn resolve(path: &str) -> Result<Arc<dyn Inode>> {
    let (root, covered) = mount::find(path)?;
    let mut node = root;
    for comp in path[covered..].split('/').filter(|c| !c.is_empty()) {
        node = node.lookup(comp)?;
    }
    Ok(node)
}

pub fn create(path: &str, kind: FileType, mode: u16) -> Result<Arc<dyn Inode>> {
    let (parent_path, name) = split_parent(path)?;
    resolve(parent_path)?.create(name, kind, mode)
}

/// `mkdir -p`
pub fn mkdir_all(path: &str, mode: u16) -> Result<()> {
    let mut current = String::new();
    for comp in path.split('/').filter(|c| !c.is_empty()) {
        current.push('/');
        current.push_str(comp);
        match resolve(&current) {
            Ok(_) => {}
            Err(Error::NotFound) => {
                create(&current, FileType::Directory, mode)?;
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// Creates (or overwrites) a whole file, making parent directories as needed.
pub fn write_file(path: &str, mode: u16, data: &[u8]) -> Result<()> {
    let (parent, _) = split_parent(path)?;
    mkdir_all(parent, 0o755)?;
    let node = match resolve(path) {
        Ok(node) => {
            node.truncate(0)?;
            node
        }
        Err(Error::NotFound) => create(path, FileType::File, mode)?,
        Err(e) => return Err(e),
    };
    node.write_at(0, data)?;
    Ok(())
}

/// unlink (files only) or rmdir (empty directories only).
pub fn remove(path: &str, want_dir: bool) -> Result<()> {
    if mount::is_mountpoint(path) {
        return Err(Error::Busy);
    }
    let (parent_path, name) = split_parent(path)?;
    let parent = resolve(parent_path)?;
    let is_dir = parent.lookup(name)?.stat()?.kind == FileType::Directory;
    match (is_dir, want_dir) {
        (true, false) => return Err(Error::IsADirectory),
        (false, true) => return Err(Error::NotADirectory),
        _ => {}
    }
    parent.unlink(name)
}

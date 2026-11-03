use super::inode::Inode;
use crate::base::sync::Mutex;
use alloc::{string::String, sync::Arc, vec::Vec};
use errors::{Error, Result};

struct Mount {
    path: String,
    fstype: &'static str,
    device: String,
    root: Arc<dyn Inode>,
}

static MOUNTS: Mutex<Vec<Mount>> = Mutex::new(Vec::new());

pub fn mount(path: &str, fstype: &'static str, device: &str, root: Arc<dyn Inode>) -> Result<()> {
    let mut mounts = MOUNTS.lock();
    if mounts.iter().any(|m| m.path == path) {
        return Err(Error::Busy);
    }
    mounts.push(Mount {
        path: String::from(path),
        fstype,
        device: String::from(device),
        root,
    });
    Ok(())
}

#[allow(dead_code)]
pub fn umount(path: &str) -> Result<()> {
    if path == "/" {
        return Err(Error::Busy);
    }
    let mut mounts = MOUNTS.lock();
    let idx = mounts
        .iter()
        .position(|m| m.path == path)
        .ok_or(Error::NotFound)?;
    mounts.remove(idx);
    Ok(())
}

pub fn is_mountpoint(path: &str) -> bool {
    MOUNTS.lock().iter().any(|m| m.path == path)
}

/// The mount that serves `path` (longest matching prefix), plus how many bytes
/// of `path` that mount covers. The rest of the path is looked up inside it.
pub fn find(path: &str) -> Result<(Arc<dyn Inode>, usize)> {
    let mounts = MOUNTS.lock();
    let mut best: Option<(&Mount, usize)> = None;
    for m in mounts.iter() {
        let p = m.path.as_str();
        let covers = p == "/"
            || path == p
            || (path.starts_with(p) && path.as_bytes().get(p.len()) == Some(&b'/'));
        if covers && best.map_or(true, |(_, score)| p.len() > score) {
            best = Some((m, p.len()));
        }
    }
    match best {
        Some((m, score)) => Ok((m.root.clone(), if m.path == "/" { 0 } else { score })),
        None => Err(Error::NotFound),
    }
}

/// (mount point, filesystem type, device) for every mount.
pub fn list() -> Vec<(String, &'static str, String)> {
    MOUNTS
        .lock()
        .iter()
        .map(|m| (m.path.clone(), m.fstype, m.device.clone()))
        .collect()
}

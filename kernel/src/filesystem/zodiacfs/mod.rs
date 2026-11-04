//! ZodiacFS: Container > Cluster > Package > Folder > Item (see FILESYSTEM.md).

pub mod allocator;
pub mod block;
pub mod container;
pub mod integrity;
pub mod layout;
pub mod node;

use super::inode::Inode;
use crate::base::sync::Mutex;
use alloc::{boxed::Box, sync::Arc};
use block::BlockDevice;
use container::Container;
use errors::Result;

/// Formats `dev` as an empty Container. DESTRUCTIVE: only call after the user
/// has explicitly confirmed; never from automatic device detection.
pub fn format(dev: &dyn BlockDevice, label: &str) -> Result<()> {
    container::format(dev, label, crate::time::now() as u64)
}

/// Validates the Container on `dev` and returns its root, ready for `mount::mount`.
/// Returns `Error::NotFound` for a disk that is not ZodiacFS (it is never formatted here).
pub fn mount(dev: Box<dyn BlockDevice>) -> Result<Arc<dyn Inode>> {
    let c = Container::mount(dev)?;
    Ok(node::ZNode::root(Arc::new(Mutex::new(c))))
}

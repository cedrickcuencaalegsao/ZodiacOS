#![allow(dead_code)]
pub mod ata;

use crate::base::error::Result;
use crate::base::sync::Mutex;
use alloc::{sync::Arc, vec::Vec};

pub trait BlockDevice: Send + Sync {
    fn name(&self) -> &str;

    fn block_size(&self) -> usize {
        512
    }

    fn block_count(&self) -> u64;

    fn read_blocks(&self, lba: u64, count: usize, buf: &mut [u8]) -> Result<()>;

    fn write_blocks(&self, lba: u64, count: usize, data: &[u8]) -> Result<()>;
}

static DEVICES: Mutex<Vec<Arc<dyn BlockDevice>>> = Mutex::new(Vec::new());

pub fn register(device: Arc<dyn BlockDevice>) {
    DEVICES.lock().push(device);
}

pub fn get(name: &str) -> Option<Arc<dyn BlockDevice>> {
    DEVICES.lock().iter().find(|d| d.name() == name).cloned()
}

pub fn list() -> Vec<Arc<dyn BlockDevice>> {
    DEVICES.lock().clone()
}

pub fn init() {
    ata::probe();
}

// kernel/src/memory/mod.rs
pub mod allocator; // placeholder: slab allocator for small kernel objects, later
pub mod heap;
pub mod physical;
pub mod virtual_memory;

pub fn init() {
    physical::init();
    virtual_memory::init();
    heap::init();
    let (total, free) = physical::stats();
    crate::log_info!("memory: {} frames allocatable, {} free", total, free);
}
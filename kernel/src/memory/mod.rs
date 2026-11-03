// kernel/src/memory/mod.rs
pub mod allocator; // placeholder: slab allocator for small kernel objects, later
pub mod heap;
pub mod physical;
pub mod virtual_memory;

use errors::Error;

pub fn init() -> Result<(), Error> {
    physical::init();
    virtual_memory::init();

    #[cfg(target_arch = "aarch64")]
    virtual_memory::set_frame_allocator(
        || physical::alloc_frame(),
        |p| physical::free_frame(p),
    );

    heap::init();
    let (total, free) = physical::stats();
    if total == 0 || free == 0 {
        return Err(Error::OutOfMemory);
    }
    crate::log_info!("memory: {} frames allocatable, {} free", total, free);
    Ok(())
}

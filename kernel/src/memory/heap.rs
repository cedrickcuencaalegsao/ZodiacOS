use super::{
    allocator, physical,
    security::Perm,
    virtual_memory::{KERNEL_SPACE, PAGE_SIZE},
};

pub const HEAP_START: u64 = 0x_4444_4444_0000;
pub const HEAP_SIZE: usize = 32 * 1024 * 1024;

/// Maps the heap's pages from fresh physical frames (RW + NX) and hands them to the allocator.
/// The pages directly below HEAP_START and above HEAP_START + HEAP_SIZE are never mapped,
/// so they act as guard pages: running off either end page-faults instead of corrupting memory.
pub fn init() {
    {
        let mut guard = KERNEL_SPACE.lock();
        let space = guard.as_mut().expect("heap: virtual memory not initialised");
        for page in 0..(HEAP_SIZE as u64 / PAGE_SIZE) {
            let frame = physical::alloc_frame().expect("heap: out of physical memory");
            space
                .map_4k(HEAP_START + page * PAGE_SIZE, frame, Perm::KERNEL_RW)
                .expect("heap: cannot map page");
        }
    } // release the VMM lock before touching the allocator

    unsafe { allocator::init(HEAP_START as usize, HEAP_SIZE) };
}
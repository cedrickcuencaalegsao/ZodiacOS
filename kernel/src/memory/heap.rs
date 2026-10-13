use crate::{kassert, paging, pmm};
use core::alloc::{GlobalAlloc, Layout};
use core::cell::UnsafeCell;
use core::ptr::null_mut;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering::{Acquire, Relaxed, Release}};

pub const HEAP_START: u64 = paging::HEAP_START;
pub const HEAP_SIZE: usize = 32 * 1024 * 1024;

const MIN_BLOCK: usize = 16; // big enough for a free-list node, keeps everything 16-byte aligned

struct Node {
    size: usize,
    next: *mut Node,
}

/// Free list sorted by address; neighbours are merged when blocks are freed.
struct Heap {
    head: *mut Node,
}

fn align_up(x: usize, a: usize) -> usize {
    (x + a - 1) & !(a - 1)
}

impl Heap {
    const fn new() -> Heap {
        Heap { head: null_mut() }
    }

    fn size_for(layout: Layout) -> usize {
        align_up(layout.size().max(MIN_BLOCK), MIN_BLOCK)
    }

    unsafe fn insert(&mut self, addr: usize, size: usize) {
        unsafe {
            let mut prev: *mut Node = null_mut();
            let mut cur = self.head;
            while !cur.is_null() && (cur as usize) < addr {
                prev = cur;
                cur = (*cur).next;
            }
            let node = addr as *mut Node;
            node.write(Node { size, next: cur });
            if prev.is_null() {
                self.head = node;
            } else {
                (*prev).next = node;
            }
            // merge with the block after it
            if !cur.is_null() && addr + (*node).size == cur as usize {
                (*node).size += (*cur).size;
                (*node).next = (*cur).next;
            }
            // merge with the block before it
            if !prev.is_null() && prev as usize + (*prev).size == addr {
                (*prev).size += (*node).size;
                (*prev).next = (*node).next;
            }
        }
    }

    unsafe fn alloc(&mut self, layout: Layout) -> *mut u8 {
        unsafe {
            let size = Self::size_for(layout);
            let align = layout.align().max(MIN_BLOCK);
            let mut link: *mut *mut Node = &mut self.head;
            let mut cur = self.head;
            while !cur.is_null() {
                let start = cur as usize;
                let end = start + (*cur).size;
                let a_start = align_up(start, align);
                if a_start + size <= end {
                    *link = (*cur).next; // unlink this block, then give back what we don't use
                    if a_start > start {
                        self.insert(start, a_start - start);
                    }
                    if a_start + size < end {
                        self.insert(a_start + size, end - (a_start + size));
                    }
                    return a_start as *mut u8;
                }
                link = &mut (*cur).next;
                cur = (*cur).next;
            }
            null_mut()
        }
    }
}

pub struct KernelHeap {
    locked: AtomicBool,
    heap: UnsafeCell<Heap>,
    used: AtomicUsize,
}

unsafe impl Sync for KernelHeap {}

impl KernelHeap {
    const fn new() -> Self {
        KernelHeap {
            locked: AtomicBool::new(false),
            heap: UnsafeCell::new(Heap::new()),
            used: AtomicUsize::new(0),
        }
    }

    fn lock(&self) {
        while self.locked.compare_exchange_weak(false, true, Acquire, Relaxed).is_err() {
            core::hint::spin_loop();
        }
    }

    fn unlock(&self) {
        self.locked.store(false, Release);
    }
}

unsafe impl GlobalAlloc for KernelHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let irq = crate::arch::save_and_disable_interrupts();
        self.lock();
        let p = unsafe { (*self.heap.get()).alloc(layout) };
        if !p.is_null() {
            self.used.fetch_add(Heap::size_for(layout), Relaxed);
        }
        self.unlock();
        crate::arch::restore_interrupts(irq);
        p
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let (start, end) = (HEAP_START as usize, HEAP_START as usize + HEAP_SIZE);
        kassert!(
            ptr as usize >= start && ptr as usize + layout.size() <= end,
            "dealloc of {:p} outside the kernel heap",
            ptr
        );
        let irq = crate::arch::save_and_disable_interrupts();
        self.lock();
        unsafe { (*self.heap.get()).insert(ptr as usize, Heap::size_for(layout)) };
        self.used.fetch_sub(Heap::size_for(layout), Relaxed);
        self.unlock();
        crate::arch::restore_interrupts(irq);
    }
}

#[global_allocator]
static ALLOCATOR: KernelHeap = KernelHeap::new();

/// Bytes currently allocated.
pub fn used() -> usize {
    ALLOCATOR.used.load(Relaxed)
}

/// Maps the heap's pages from fresh physical frames and hands them to the allocator.
pub fn init() {
    let pml4 = paging::kernel_pml4();
    for page in 0..(HEAP_SIZE / 4096) as u64 {
        let frame = pmm::alloc_frame().expect("heap: out of physical memory");
        paging::map_page(pml4, HEAP_START + page * 4096, frame, paging::WRITABLE | paging::NX)
            .expect("heap: cannot map page");
    }
    ALLOCATOR.lock();
    unsafe { (*ALLOCATOR.heap.get()).insert(HEAP_START as usize, HEAP_SIZE) };
    ALLOCATOR.unlock();
}
#![allow(dead_code)]
use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{
    AtomicBool,
    Ordering::{Acquire, Relaxed, Release},
};

// ---- SpinLock: busy-waits. Never hold it across anything that can sleep. -------
pub struct SpinLock<T> {
    locked: AtomicBool,
    data: UnsafeCell<T>,
}

unsafe impl<T: Send> Sync for SpinLock<T> {}

impl<T> SpinLock<T> {
    pub const fn new(value: T) -> Self {
        SpinLock {
            locked: AtomicBool::new(false),
            data: UnsafeCell::new(value),
        }
    }

    pub fn lock(&self) -> SpinGuard<'_, T> {
        while self
            .locked
            .compare_exchange_weak(false, true, Acquire, Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }
        SpinGuard { lock: self }
    }

    pub fn try_lock(&self) -> Option<SpinGuard<'_, T>> {
        if self
            .locked
            .compare_exchange(false, true, Acquire, Relaxed)
            .is_ok()
        {
            Some(SpinGuard { lock: self })
        } else {
            None
        }
    }
}

pub struct SpinGuard<'a, T> {
    lock: &'a SpinLock<T>,
}

impl<T> Deref for SpinGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { &*self.lock.data.get() }
    }
}

impl<T> DerefMut for SpinGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<T> Drop for SpinGuard<'_, T> {
    fn drop(&mut self) {
        self.lock.locked.store(false, Release);
    }
}

// ---- Locks that need the scheduler and interrupt control (x86_64 for now) --------
#[cfg(target_arch = "x86_64")]
mod blocking {
    use super::{SpinGuard, SpinLock};
    use crate::scheduler::{self, run_queue::RunQueue};
    use core::cell::UnsafeCell;
    use core::mem::ManuallyDrop;
    use core::ops::{Deref, DerefMut};
    use shared::kassert;

    /// SpinLock that also disables interrupts while held. Required for any data an
    /// interrupt handler or the scheduler touches: otherwise a tick could preempt
    /// the holder and the next thread would spin forever on the lock.
    pub struct IrqSpinLock<T> {
        inner: SpinLock<T>,
    }

    impl<T> IrqSpinLock<T> {
        pub const fn new(value: T) -> Self {
            IrqSpinLock {
                inner: SpinLock::new(value),
            }
        }

        pub fn lock(&self) -> IrqGuard<'_, T> {
            let was_enabled = crate::arch::save_and_disable_interrupts();
            IrqGuard {
                guard: ManuallyDrop::new(self.inner.lock()),
                was_enabled,
            }
        }

        /// The plain lock, for the scheduler, which manages interrupts by hand.
        pub fn raw(&self) -> &SpinLock<T> {
            &self.inner
        }
    }

    pub struct IrqGuard<'a, T> {
        guard: ManuallyDrop<SpinGuard<'a, T>>,
        was_enabled: bool,
    }

    impl<T> Deref for IrqGuard<'_, T> {
        type Target = T;
        fn deref(&self) -> &T {
            &self.guard
        }
    }

    impl<T> DerefMut for IrqGuard<'_, T> {
        fn deref_mut(&mut self) -> &mut T {
            &mut self.guard
        }
    }

    impl<T> Drop for IrqGuard<'_, T> {
        fn drop(&mut self) {
            unsafe { ManuallyDrop::drop(&mut self.guard) }; // release the lock first
            crate::arch::restore_interrupts(self.was_enabled);
        }
    }

    // ---- Mutex: a contended lock puts the caller to sleep instead of spinning -----
    struct MutexState {
        locked: bool,
        waiters: RunQueue,
    }

    pub struct Mutex<T> {
        state: IrqSpinLock<MutexState>,
        data: UnsafeCell<T>,
    }

    unsafe impl<T: Send> Sync for Mutex<T> {}

    impl<T> Mutex<T> {
        pub const fn new(value: T) -> Self {
            Mutex {
                state: IrqSpinLock::new(MutexState {
                    locked: false,
                    waiters: RunQueue::new(),
                }),
                data: UnsafeCell::new(value),
            }
        }

        pub fn lock(&self) -> MutexGuard<'_, T> {
            loop {
                {
                    let mut st = self.state.lock();
                    if !st.locked {
                        st.locked = true;
                        return MutexGuard { mutex: self };
                    }
                    kassert!(
                        scheduler::ready(),
                        "contended Mutex before the scheduler started"
                    );
                    st.waiters.push(scheduler::current_slot());
                    scheduler::block_current();
                } // interrupts back on; the owner's unlock will wake us
                scheduler::schedule();
            }
        }

        pub fn try_lock(&self) -> Option<MutexGuard<'_, T>> {
            let mut st = self.state.lock();
            if st.locked {
                None
            } else {
                st.locked = true;
                Some(MutexGuard { mutex: self })
            }
        }
    }

    pub struct MutexGuard<'a, T> {
        mutex: &'a Mutex<T>,
    }

    impl<T> Deref for MutexGuard<'_, T> {
        type Target = T;
        fn deref(&self) -> &T {
            unsafe { &*self.mutex.data.get() }
        }
    }

    impl<T> DerefMut for MutexGuard<'_, T> {
        fn deref_mut(&mut self) -> &mut T {
            unsafe { &mut *self.mutex.data.get() }
        }
    }

    impl<T> Drop for MutexGuard<'_, T> {
        fn drop(&mut self) {
            let mut st = self.mutex.state.lock();
            st.locked = false;
            if let Some(slot) = st.waiters.pop() {
                scheduler::wake_slot(slot);
            }
        }
    }
}

#[cfg(target_arch = "x86_64")]
pub use blocking::{IrqSpinLock, Mutex};

use core::fmt;
use core::sync::atomic::{AtomicBool, Ordering::SeqCst};

static ASSERTION: AtomicBool = AtomicBool::new(false);

/// Used by the panic handler to choose between "KERNEL PANIC" and the assertion title.
pub fn take_flag() -> bool {
    ASSERTION.swap(false, SeqCst)
}

/// `#[track_caller]` makes the panic report the line of the `kassert!` call,
/// not a line inside this file.
#[track_caller]
#[cold]
pub fn fail(args: fmt::Arguments) -> ! {
    ASSERTION.store(true, SeqCst);
    panic!("{}", args)
}

/// Checks a condition that must always be true. Optional message uses format syntax.
///   kassert!(len <= 512);
///   kassert!(id < 4, "bad drive id {}", id);
#[macro_export]
macro_rules! kassert {
    ($cond:expr $(,)?) => {
        if !$cond {
            $crate::assertions::fail(format_args!("assertion failed: {}", stringify!($cond)));
        }
    };
    ($cond:expr, $($arg:tt)+) => {
        if !$cond {
            $crate::assertions::fail(format_args!(
                "assertion failed: {}: {}",
                stringify!($cond),
                format_args!($($arg)+)
            ));
        }
    };
}

#[macro_export]
macro_rules! kassert_eq {
    ($left:expr, $right:expr $(,)?) => {
        match (&$left, &$right) {
            (l, r) => {
                if !(*l == *r) {
                    $crate::assertions::fail(format_args!(
                        "assertion failed: left == right\n  left:  {:?}\n  right: {:?}",
                        l, r
                    ));
                }
            }
        }
    };
}

#[macro_export]
macro_rules! kassert_ne {
    ($left:expr, $right:expr $(,)?) => {
        match (&$left, &$right) {
            (l, r) => {
                if *l == *r {
                    $crate::assertions::fail(format_args!(
                        "assertion failed: left != right\n  both:  {:?}",
                        l
                    ));
                }
            }
        }
    };
}

/// Same as `kassert!`, but compiled out unless `debug-assertions` is on.
/// Use it for checks that are too slow for hot paths.
#[macro_export]
macro_rules! kdebug_assert {
    ($($arg:tt)*) => {
        if cfg!(debug_assertions) {
            $crate::kassert!($($arg)*);
        }
    };
}
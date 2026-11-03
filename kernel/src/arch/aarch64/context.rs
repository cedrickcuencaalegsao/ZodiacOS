//! arch/aarch64/context.rs
//!
//! Kernel-thread context switch. Only the callee-saved registers need saving, because
//! `zodiac_switch` is an ordinary function call as far as the compiler is concerned:
//!
//!   x19..x28   callee-saved general registers
//!   x29 (fp), x30 (lr)
//!   sp         stored in the Task (the field the x86 code calls `rsp`)
//!
//! Saved frame layout, 96 bytes, lowest address first:
//!   [ x19 x20 | x21 x22 | x23 x24 | x25 x26 | x27 x28 | x29 x30 ]
//!
//! `ret` at the end of the switch jumps to the restored x30. For a task that has
//! never run, `init_stack` plants `zodiac_task_start` there.

use core::arch::global_asm;

pub const FRAME_SIZE: usize = 96;

global_asm!(
    r#"
.balign 4
.global zodiac_switch
zodiac_switch:
    // x0 = *mut u64 (where to store the old sp), x1 = new sp
    sub  sp, sp, #96
    stp  x19, x20, [sp, #0]
    stp  x21, x22, [sp, #16]
    stp  x23, x24, [sp, #32]
    stp  x25, x26, [sp, #48]
    stp  x27, x28, [sp, #64]
    stp  x29, x30, [sp, #80]
    mov  x2, sp
    str  x2, [x0]
    mov  sp, x1
    ldp  x19, x20, [sp, #0]
    ldp  x21, x22, [sp, #16]
    ldp  x23, x24, [sp, #32]
    ldp  x25, x26, [sp, #48]
    ldp  x27, x28, [sp, #64]
    ldp  x29, x30, [sp, #80]
    add  sp, sp, #96
    ret

// First code a new task runs. x19 = the task's entry function (set by init_stack).
// We arrive from inside schedule(), possibly from the timer interrupt, so IRQs are
// still masked: unmask them before running the task body.
.balign 4
.global zodiac_task_start
zodiac_task_start:
    msr  daifclr, #2
    blr  x19
1:  wfi                     // entry must never return
    b    1b
"#
);

unsafe extern "C" {
    /// Saves the current context, stores its sp in `*old_sp`, and resumes the one whose
    /// saved sp is `new_sp`.
    pub fn zodiac_switch(old_sp: *mut u64, new_sp: u64);
    fn zodiac_task_start();
}

/// Builds the initial frame for a task that has never run and returns the sp to store in
/// the Task. `kstack_top` is the (exclusive) top of its kernel stack. `entry` is the
/// function the task starts in (the same trampoline the x86 code uses to read the Task's
/// `Entry` and call it).
pub fn init_stack(kstack_top: u64, entry: extern "C" fn() -> !) -> u64 {
    let sp = (kstack_top - FRAME_SIZE as u64) & !0xF; // sp must stay 16-byte aligned
    let frame = sp as *mut u64;
    unsafe {
        for i in 0..(FRAME_SIZE / 8) {
            frame.add(i).write(0);
        }
        frame.add(0).write(entry as usize as u64); // x19
        frame.add(11).write(zodiac_task_start as usize as u64); // x30 (lr)
        // x29 stays 0 so a backtrace stops here
    }
    sp
}
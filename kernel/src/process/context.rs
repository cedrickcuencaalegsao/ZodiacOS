use core::arch::global_asm;

global_asm!(
    r#"
.global zodiac_switch
// void zodiac_switch(u64 *old_rsp, u64 new_rsp)
// Saves the callee-saved registers on the current stack, swaps stacks, restores
// the next task's registers. Everything else is already saved by the compiler.
zodiac_switch:
    push rbp
    push rbx
    push r12
    push r13
    push r14
    push r15
    mov  [rdi], rsp
    mov  rsp, rsi
    pop  r15
    pop  r14
    pop  r13
    pop  r12
    pop  rbx
    pop  rbp
    ret

.global zodiac_enter_user
// void zodiac_enter_user(u64 rip, u64 rsp) -> !
// Builds an interrupt frame and "returns" to ring 3.
zodiac_enter_user:
    cli
    push 0x1B               // ss: user data selector
    push rsi                // rsp: user stack pointer
    push 0x202              // rflags: interrupts on
    push 0x23               // cs: user code selector
    push rdi                // rip
    iretq
"#
);

unsafe extern "C" {
    pub fn zodiac_switch(old_rsp: *mut u64, new_rsp: u64);
    pub fn zodiac_enter_user(rip: u64, rsp: u64) -> !;
}
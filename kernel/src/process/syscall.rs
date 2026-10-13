use crate::base::error::Error;
use crate::{arch, interrupts, paging, scheduler, vga};
use core::arch::global_asm;

pub const SYS_WRITE: u64 = 0; // write(fd, ptr, len)
pub const SYS_EXIT: u64 = 1; // exit(code)
pub const SYS_YIELD: u64 = 2; // yield()
pub const SYS_SLEEP: u64 = 3; // sleep(ms)
pub const SYS_GETPID: u64 = 4; // getpid()
pub const SYS_UPTIME: u64 = 5; // uptime_ms()

global_asm!(
    r#"
.global zodiac_syscall_entry
zodiac_syscall_entry:
    cld                     // user code may have set the direction flag
    mov  rcx, rdx           // a2
    mov  rdx, rsi           // a1
    mov  rsi, rdi           // a0
    mov  rdi, rax           // syscall number
    sti                     // system calls can be preempted
    sub  rsp, 8             // keep the stack 16-byte aligned for the call
    call syscall_dispatch
    add  rsp, 8
    cli
    iretq                   // back to ring 3, result in rax
"#
);

unsafe extern "C" {
    fn zodiac_syscall_entry();
}

pub fn init() {
    crate::idt::set_gate_user(0x80, zodiac_syscall_entry as *const () as u64);
}

#[unsafe(no_mangle)]
pub extern "C" fn syscall_dispatch(num: u64, a0: u64, a1: u64, a2: u64) -> u64 {
    let result: Result<u64, Error> = match num {
        SYS_WRITE => sys_write(a0, a1, a2),
        SYS_EXIT => crate::process::exit_current(a0 as i32),
        SYS_YIELD => {
            scheduler::yield_now();
            Ok(0)
        }
        SYS_SLEEP => {
            scheduler::sleep_ms(a0.min(10_000));
            Ok(0)
        }
        SYS_GETPID => Ok(scheduler::current_pid() as u64),
        SYS_UPTIME => Ok(interrupts::uptime_ms()),
        _ => Err(Error::Unsupported),
    };
    match result {
        Ok(value) => value,
        Err(e) => (-(e.errno() as i64)) as u64,
    }
}

/// A user pointer is only trusted if the whole range lies in user space and every
/// page in it is mapped user-accessible. This is what stops a program from asking
/// the kernel to read or write kernel memory on its behalf.
fn check_user_buffer(ptr: u64, len: u64) -> Result<(), Error> {
    if len == 0 {
        return Ok(());
    }
    let end = ptr.checked_add(len).ok_or(Error::BadAddress)?;
    if ptr < paging::USER_BASE || end > paging::USER_END {
        return Err(Error::BadAddress);
    }
    let pml4 = paging::current();
    let mut page = ptr & !0xFFF;
    while page < end {
        match paging::translate(pml4, page) {
            Some(m) if m.user => {}
            _ => return Err(Error::BadAddress),
        }
        page += 4096;
    }
    Ok(())
}

fn sys_write(fd: u64, ptr: u64, len: u64) -> Result<u64, Error> {
    if fd != 1 && fd != 2 {
        return Err(Error::BadFileDescriptor);
    }
    let len = len.min(512);
    check_user_buffer(ptr, len)?;

    // copy first, so the program can't change the bytes while we print them
    let mut buf = [0u8; 512];
    unsafe { core::ptr::copy_nonoverlapping(ptr as *const u8, buf.as_mut_ptr(), len as usize) };

    let was_enabled = arch::save_and_disable_interrupts();
    for &b in &buf[..len as usize] {
        vga::put_byte(b);
    }
    arch::restore_interrupts(was_enabled);
    Ok(len)
}
use crate::base::error::Error;
use crate::filesystem::file;
use crate::{interrupts, paging, scheduler};
use alloc::{string::String, vec::Vec};
use core::arch::global_asm;

pub const SYS_WRITE: u64 = 0; // write(fd, ptr, len)
pub const SYS_EXIT: u64 = 1; // exit(code)
pub const SYS_YIELD: u64 = 2; // yield()
pub const SYS_SLEEP: u64 = 3; // sleep(ms)
pub const SYS_GETPID: u64 = 4; // getpid()
pub const SYS_UPTIME: u64 = 5; // uptime_ms()
pub const SYS_OPEN: u64 = 6; // open(path, flags, mode) -> fd
pub const SYS_CLOSE: u64 = 7; // close(fd)
pub const SYS_READ: u64 = 8; // read(fd, ptr, len)
pub const SYS_LSEEK: u64 = 9; // lseek(fd, offset, whence)
pub const SYS_MKDIR: u64 = 10; // mkdir(path, mode)
pub const SYS_UNLINK: u64 = 11; // unlink(path)

const MAX_PATH_ARG: usize = 256;

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
        SYS_OPEN => path_from_user(a0)
            .and_then(|p| file::open(&p, a1 as u32, a2 as u16))
            .map(|fd| fd as u64),
        SYS_CLOSE => file::close(a0 as usize).map(|_| 0),
        SYS_READ => sys_read(a0, a1, a2),
        SYS_LSEEK => file::lseek(a0 as usize, a1 as i64, a2 as u32),
        SYS_MKDIR => path_from_user(a0)
            .and_then(|p| file::mkdir(&p, a1 as u16))
            .map(|_| 0),
        SYS_UNLINK => path_from_user(a0).and_then(|p| file::unlink(&p)).map(|_| 0),
        _ => Err(Error::Unsupported),
    };
    match result {
        Ok(value) => value,
        Err(e) => (-(e.errno() as i64)) as u64,
    }
}

/// A user pointer is only trusted if the whole range lies in user space and every
/// page in it is mapped user-accessible (and writable, when the kernel will write
/// into it). This is what stops a program asking the kernel to read or write
/// kernel memory on its behalf.
fn check_user_buffer(ptr: u64, len: u64, writable: bool) -> Result<(), Error> {
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
            Some(m) if m.user && (!writable || m.writable) => {}
            _ => return Err(Error::BadAddress),
        }
        page += 4096;
    }
    Ok(())
}

/// Copies a NUL-terminated string out of user memory, checking each page as it goes.
fn path_from_user(ptr: u64) -> Result<String, Error> {
    let mut bytes: Vec<u8> = Vec::new();
    for i in 0..MAX_PATH_ARG as u64 {
        let addr = ptr.checked_add(i).ok_or(Error::BadAddress)?;
        if i == 0 || addr % 4096 == 0 {
            check_user_buffer(addr, 1, false)?;
        }
        let byte = unsafe { *(addr as *const u8) };
        if byte == 0 {
            return String::from_utf8(bytes).map_err(|_| Error::InvalidArgument);
        }
        bytes.push(byte);
    }
    Err(Error::NameTooLong)
}

fn sys_write(fd: u64, ptr: u64, len: u64) -> Result<u64, Error> {
    let len = len.min(65536);
    check_user_buffer(ptr, len, false)?;
    let mut chunk = [0u8; 512];
    let mut total = 0u64;
    while total < len {
        let n = ((len - total) as usize).min(512);
        // copy first, so the program can't change the bytes while we use them
        unsafe {
            core::ptr::copy_nonoverlapping((ptr + total) as *const u8, chunk.as_mut_ptr(), n)
        };
        match file::write(fd as usize, &chunk[..n]) {
            Ok(written) => {
                total += written as u64;
                if written < n {
                    break;
                }
            }
            Err(e) => {
                if total == 0 {
                    return Err(e);
                }
                break;
            }
        }
    }
    Ok(total)
}

fn sys_read(fd: u64, ptr: u64, len: u64) -> Result<u64, Error> {
    let len = len.min(65536);
    check_user_buffer(ptr, len, true)?;
    let mut chunk = [0u8; 512];
    let mut total = 0u64;
    while total < len {
        let want = ((len - total) as usize).min(512);
        let n = match file::read(fd as usize, &mut chunk[..want]) {
            Ok(n) => n,
            Err(e) => {
                if total == 0 {
                    return Err(e);
                }
                break;
            }
        };
        if n == 0 {
            break;
        }
        unsafe { core::ptr::copy_nonoverlapping(chunk.as_ptr(), (ptr + total) as *mut u8, n) };
        total += n as u64;
        if n < want {
            break;
        }
    }
    Ok(total)
}

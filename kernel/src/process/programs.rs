use core::arch::global_asm;

global_asm!(
    r#"
.section .rodata.userprog, "a"

// hello: write a message, then exit with its own pid as the exit code
.global zodiac_prog_hello_start
.global zodiac_prog_hello_end
zodiac_prog_hello_start:
    mov  eax, 0             // write(1, msg, 22)
    mov  edi, 1
    lea  rsi, [rip + 2f]
    mov  edx, 22
    int  0x80
    mov  eax, 4             // getpid()
    int  0x80
    mov  edi, eax           // exit(pid)
    mov  eax, 1
    int  0x80
2:  .ascii "Hello from user mode!"
    .byte 10
zodiac_prog_hello_end:

// read: tries to read kernel memory directly -> page fault in user mode
.global zodiac_prog_read_start
.global zodiac_prog_read_end
zodiac_prog_read_start:
    mov  rax, 0x10000       // an address inside the kernel image
    mov  rbx, [rax]
    mov  eax, 1             // not reached
    xor  edi, edi
    int  0x80
zodiac_prog_read_end:

// kptr: asks the kernel to print kernel memory -> the kernel must refuse
.global zodiac_prog_kptr_start
.global zodiac_prog_kptr_end
zodiac_prog_kptr_start:
    mov  eax, 0             // write(1, 0x10000, 16)
    mov  edi, 1
    mov  esi, 0x10000
    mov  edx, 16
    int  0x80
    mov  edi, eax           // exit with the (negative) error code
    mov  eax, 1
    int  0x80
zodiac_prog_kptr_end:

// priv: executes a privileged instruction -> #GP in user mode
.global zodiac_prog_priv_start
.global zodiac_prog_priv_end
zodiac_prog_priv_start:
    hlt
    mov  eax, 1
    xor  edi, edi
    int  0x80
zodiac_prog_priv_end:

.text
"#
);

unsafe extern "C" {
    static zodiac_prog_hello_start: u8;
    static zodiac_prog_hello_end: u8;
    static zodiac_prog_read_start: u8;
    static zodiac_prog_read_end: u8;
    static zodiac_prog_kptr_start: u8;
    static zodiac_prog_kptr_end: u8;
    static zodiac_prog_priv_start: u8;
    static zodiac_prog_priv_end: u8;
}

fn blob(start: *const u8, end: *const u8) -> &'static [u8] {
    unsafe { core::slice::from_raw_parts(start, end as usize - start as usize) }
}

pub fn get(name: &str) -> Option<&'static [u8]> {
    match name {
        "hello" => Some(blob(&raw const zodiac_prog_hello_start, &raw const zodiac_prog_hello_end)),
        "read" => Some(blob(&raw const zodiac_prog_read_start, &raw const zodiac_prog_read_end)),
        "kptr" => Some(blob(&raw const zodiac_prog_kptr_start, &raw const zodiac_prog_kptr_end)),
        "priv" => Some(blob(&raw const zodiac_prog_priv_start, &raw const zodiac_prog_priv_end)),
        _ => None,
    }
}
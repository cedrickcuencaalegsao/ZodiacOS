#![allow(dead_code)]
unsafe extern "C" {
    static __kernel_start: u8;
    static __kernel_end: u8;
    static __text_start: u8;
    static __text_end: u8;
    static __rodata_start: u8;
    static __rodata_end: u8;
    static __data_start: u8;
}

macro_rules! sym {
    ($name:ident, $s:ident) => {
        pub fn $name() -> u64 {
            core::ptr::addr_of!($s) as u64
        }
    };
}

sym!(kernel_start, __kernel_start);
sym!(kernel_end, __kernel_end);
sym!(text_start, __text_start);
sym!(text_end, __text_end);
sym!(rodata_start, __rodata_start);
sym!(rodata_end, __rodata_end);
sym!(data_start, __data_start);

pub fn align_down(x: u64, a: u64) -> u64 {
    x & !(a - 1)
}
pub fn align_up(x: u64, a: u64) -> u64 {
    (x + a - 1) & !(a - 1)
}

use crate::arch::sleep_ms;
use crate::vga::{self, print_at, put_char};

pub fn run(total_ms: u32) {
    const BAR_LEN: usize = 20;
    const SPINNER: [u8; 4] = [b'|', b'/', b'-', b'\\'];

    let row = vga::height() / 2;
    let col = (vga::width() - (BAR_LEN + 2)) / 2;

    print_at(row, col, b"Loading ZodiacOS", 0x0F);
    put_char(row + 1, col, b'[', 0x07);
    put_char(row + 1, col + BAR_LEN + 1, b']', 0x07);

    let step_ms = total_ms / (BAR_LEN as u32 * 3);
    let mut frame = 0;

    for i in 0..BAR_LEN {
        for _ in 0..3 {
            put_char(row, col + 17, SPINNER[frame % 4], 0x0E);
            frame += 1;
            sleep_ms(step_ms);
        }
        put_char(row + 1, col + 1 + i, 0xDB, 0x0A);
    }
}
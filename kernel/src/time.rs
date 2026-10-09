use crate::arch::{inb, outb};
use core::fmt;

fn cmos(reg: u8) -> u8 {
    unsafe {
        outb(0x70, reg);
        inb(0x71)
    }
}

fn bcd(v: u8) -> u8 {
    (v & 0x0F) + (v >> 4) * 10
}

/// Current time as seconds since 1970-01-01, read from the RTC.
pub fn now() -> u32 {
    let mut spins = 0;
    while cmos(0x0A) & 0x80 != 0 && spins < 100_000 {
        spins += 1; // wait out an in-progress clock update
    }

    let b = cmos(0x0B);
    let is_bcd = b & 0x04 == 0;
    let conv = |v: u8| if is_bcd { bcd(v) } else { v };

    let s = conv(cmos(0x00)) as i64;
    let m = conv(cmos(0x02)) as i64;
    let d = conv(cmos(0x07)) as u32;
    let mo = conv(cmos(0x08)) as u32;
    let y = 2000 + conv(cmos(0x09)) as i32;

    let raw_h = cmos(0x04);
    let pm = raw_h & 0x80 != 0;
    let mut h = conv(raw_h & 0x7F) as i64;
    if b & 0x02 == 0 {
        // 12-hour mode
        h = h % 12 + if pm { 12 } else { 0 };
    }

    let days = days_from_civil(y, mo, d) as i64;
    (days * 86_400 + h * 3600 + m * 60 + s) as u32
}

fn days_from_civil(y: i32, m: u32, d: u32) -> i32 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = (y - era * 400) as u32;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe as i32 - 719_468
}

fn civil_from_days(z: i32) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i32 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Prints a timestamp as `YYYY-MM-DD HH:MM` (always 16 characters wide).
pub struct Stamp(pub u32);

impl fmt::Display for Stamp {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.0 == 0 {
            return write!(f, "{:<16}", "-");
        }
        let (y, mo, d) = civil_from_days((self.0 / 86_400) as i32);
        let secs = self.0 % 86_400;
        write!(f, "{:04}-{:02}-{:02} {:02}:{:02}", y, mo, d, secs / 3600, secs % 3600 / 60)
    }
}
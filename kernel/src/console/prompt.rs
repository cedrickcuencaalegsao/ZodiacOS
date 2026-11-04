#![allow(dead_code)]
use crate::keyboard;
use crate::{kprint, kprintln};
use alloc::string::String;

fn read_input(label: &str, echo: bool) -> String {
    keyboard::flush(); // drop stray keys buffered before the prompt
    kprint!("{label}");
    let mut buf = String::new();
    loop {
        match keyboard::wait_char() {
            b'\n' => {
                kprintln!();
                return buf;
            }
            8 => {
                // Backspace (serial's 0x7F is already translated to 8)
                if buf.pop().is_some() && echo {
                    kprint!("\x08 \x08");
                }
            }
            // Printable ASCII only; this also ignores the 0x80+ arrow/Home/End/Delete codes
            c @ 0x20..=0x7E => {
                buf.push(c as char);
                if echo {
                    kprint!("{}", c as char);
                }
            }
            _ => {}
        }
    }
}

/// Reads a line without echoing it.
pub fn read_password(label: &str) -> String {
    read_input(label, false)
}

/// Reads a line with normal echo (for the username).
pub fn read_line(label: &str) -> String {
    read_input(label, true)
}
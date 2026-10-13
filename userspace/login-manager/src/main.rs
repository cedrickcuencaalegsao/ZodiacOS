// mod authentication;
// mod greeter;
// mod session;
// mod user_accounts;

// use greeter::{Key, Tty};

// struct ZTty;  // wraps libzodiac syscalls

// impl Tty for ZTty {
//     fn size(&self) -> (usize, usize)                { /* sys_console_size() */ }
//     fn put(&mut self, r: usize, c: usize, ch: u8, a: u8) { /* sys_put_char(r,c,ch,a) */ }
//     fn read_key(&mut self) -> Key                   { /* blocking sys_read_key() → Key */ }
//     fn poll_key(&mut self) -> Option<Key>           { /* non-blocking variant */ }
//     fn sleep_ms(&mut self, ms: u32)                 { /* sys_sleep_ms(ms) */ }
//     fn clear(&mut self)                             { /* sys_clear() */ }
// }

// fn main() {
//     let mut tty = ZTty;
//     loop {
//         greeter::run(&mut tty);
//         match authentication::run(&mut tty) {
//             authentication::AuthResult::Granted { .. } => {
//                 // session::start(user) → launches shell/desktop; returns on logout
//                 // after logout the loop restarts at the greeter
//             }
//             authentication::AuthResult::Back => continue, // Esc → back to greeter
//         }
//     }
// }
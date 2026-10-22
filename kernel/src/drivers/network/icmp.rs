use super::ipv4::{self, checksum};
use crate::base::error::Result;
use crate::drivers::network::NetDevice;
use crate::interrupts;
use alloc::{sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicU16, AtomicU32, AtomicU64, Ordering::SeqCst};

const ID: u16 = 0x5A44;

static NEXT_SEQ: AtomicU16 = AtomicU16::new(1);
static LAST_REPLY: AtomicU32 = AtomicU32::new(0); // (id << 16) | sequence of the latest echo reply
static REPLY_TICK: AtomicU64 = AtomicU64::new(0); // when it arrived

pub fn next_seq() -> u16 {
    NEXT_SEQ.fetch_add(1, SeqCst)
}

/// Sends an echo request ("ping") with a 32-byte payload.
pub fn send_echo(dev: &Arc<dyn NetDevice>, dst: [u8; 4], seq: u16) -> Result<()> {
    let mut msg = Vec::with_capacity(40);
    msg.extend_from_slice(&[8, 0, 0, 0]); // type 8 = echo request, code 0, checksum later
    msg.extend_from_slice(&ID.to_be_bytes());
    msg.extend_from_slice(&seq.to_be_bytes());
    for i in 0..32u8 {
        msg.push(b'a' + i % 26);
    }
    let sum = checksum(&msg);
    msg[2..4].copy_from_slice(&sum.to_be_bytes());
    ipv4::send(dev, dst, ipv4::ICMP, &msg)
}

/// The tick an echo reply for `seq` arrived at, if it has.
pub fn reply_tick(seq: u16) -> Option<u64> {
    if LAST_REPLY.load(SeqCst) == (ID as u32) << 16 | seq as u32 {
        Some(REPLY_TICK.load(SeqCst))
    } else {
        None
    }
}

pub fn handle(dev: &Arc<dyn NetDevice>, src_mac: [u8; 6], src_ip: [u8; 4], msg: &[u8]) {
    if msg.len() < 8 || checksum(msg) != 0 {
        return;
    }
    match msg[0] {
        8 => {
            // someone pinged us: answer with the same payload
            let mut reply = msg.to_vec();
            reply[0] = 0;
            reply[2] = 0;
            reply[3] = 0;
            let sum = checksum(&reply);
            reply[2..4].copy_from_slice(&sum.to_be_bytes());
            let _ = ipv4::send_to_mac(dev, src_mac, src_ip, ipv4::ICMP, &reply);
        }
        0 => {
            let id = u16::from_be_bytes([msg[4], msg[5]]);
            let seq = u16::from_be_bytes([msg[6], msg[7]]);
            if id == ID {
                REPLY_TICK.store(interrupts::ticks(), SeqCst);
                LAST_REPLY.store((id as u32) << 16 | seq as u32, SeqCst);
            }
        }
        _ => {}
    }
}
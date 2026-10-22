use super::{arp, config, ethernet, icmp};
use crate::drivers::network::NetDevice;
use alloc::{sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicU16, Ordering::Relaxed};
use errors::{Error, Result};

pub const ICMP: u8 = 1;

static NEXT_ID: AtomicU16 = AtomicU16::new(1);

/// The Internet checksum: the one's complement of the one's complement sum of 16-bit
/// words. Over a valid packet (checksum field included) the result is 0.
pub fn checksum(data: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    let mut words = data.chunks_exact(2);
    for w in &mut words {
        sum += u16::from_be_bytes([w[0], w[1]]) as u32;
    }
    if let [last] = words.remainder() {
        sum += (*last as u32) << 8;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !(sum as u16)
}

pub fn same_subnet(a: [u8; 4], b: [u8; 4], mask: [u8; 4]) -> bool {
    (0..4).all(|i| a[i] & mask[i] == b[i] & mask[i])
}

fn build_packet(src: [u8; 4], dst: [u8; 4], protocol: u8, payload: &[u8]) -> Vec<u8> {
    let total = 20 + payload.len();
    let mut p = Vec::with_capacity(total);
    p.extend_from_slice(&[0x45, 0]); // version 4, 20-byte header, no special service
    p.extend_from_slice(&(total as u16).to_be_bytes());
    p.extend_from_slice(&NEXT_ID.fetch_add(1, Relaxed).to_be_bytes());
    p.extend_from_slice(&[0x40, 0]); // don't fragment
    p.extend_from_slice(&[64, protocol, 0, 0]); // time to live, protocol, checksum (filled in below)
    p.extend_from_slice(&src);
    p.extend_from_slice(&dst);
    let sum = checksum(&p[..20]);
    p[10..12].copy_from_slice(&sum.to_be_bytes());
    p.extend_from_slice(payload);
    p
}

/// Sends an IPv4 packet, finding the destination MAC with ARP (this can wait for the reply).
pub fn send(dev: &Arc<dyn NetDevice>, dst: [u8; 4], protocol: u8, payload: &[u8]) -> Result<()> {
    let mac = arp::resolve(dev, dst)?;
    send_to_mac(dev, mac, dst, protocol, payload)
}

/// Sends to a MAC address we already know, so it never waits. The network thread uses
/// this to answer packets, because it can't wait for ARP replies it has to read itself.
pub fn send_to_mac(
    dev: &Arc<dyn NetDevice>,
    mac: [u8; 6],
    dst: [u8; 4],
    protocol: u8,
    payload: &[u8],
) -> Result<()> {
    if payload.len() > 1480 {
        return Err(Error::InvalidArgument);
    }
    let packet = build_packet(config().ip, dst, protocol, payload);
    dev.send(&ethernet::build(mac, dev.mac(), ethernet::IPV4, &packet))
}

/// Called by the network thread for every IPv4 frame addressed to us.
pub fn handle(dev: &Arc<dyn NetDevice>, src_mac: [u8; 6], packet: &[u8]) {
    if packet.len() < 20 || packet[0] >> 4 != 4 {
        return;
    }
    let header_len = (packet[0] & 0x0F) as usize * 4;
    let total = u16::from_be_bytes([packet[2], packet[3]]) as usize;
    if header_len < 20 || total < header_len || total > packet.len() {
        return;
    }
    if checksum(&packet[..header_len]) != 0 {
        return; // damaged header
    }
    if u16::from_be_bytes([packet[6], packet[7]]) & 0x3FFF != 0 {
        return; // fragmented: not reassembled
    }
    let src: [u8; 4] = packet[12..16].try_into().unwrap();
    let dst: [u8; 4] = packet[16..20].try_into().unwrap();
    if dst != config().ip && dst != [255; 4] {
        return;
    }
    let payload = &packet[header_len..total];
    if packet[9] == ICMP {
        icmp::handle(dev, src_mac, src, payload);
    }
}

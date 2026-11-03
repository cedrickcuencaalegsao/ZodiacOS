//! Minimal DNS client: looks up A records (IPv4 addresses) over UDP.
//!
//! Self-contained on purpose: it builds its own Ethernet + IPv4 + UDP frame and
//! gets replies through `handle_udp()`, which `net_thread` calls for every IPv4
//! frame before the normal IPv4 handler. A general UDP layer can replace this later.
//!
//! QEMU's user network runs a DNS forwarder at 10.0.2.3 that answers for the
//! whole internet, so this works for google.com, youtube.com, github.com, etc.

use super::{arp, config, interface, Config, MacAddr, NetDevice};
use crate::base::sync::Mutex;
use crate::{interrupts, scheduler};
use alloc::{sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicU16, Ordering::Relaxed};

/// QEMU's built-in DNS server. Change this (or read it from DHCP later) on other networks.
pub const DNS_SERVER: [u8; 4] = [10, 0, 2, 3];
const DNS_PORT: u16 = 53;

/// The query we are waiting on, and its reply once `handle_udp` has caught it.
struct Pending {
    id: u16,
    sport: u16,
    reply: Option<Vec<u8>>,
}

static PENDING: Mutex<Pending> = Mutex::new(Pending {
    id: 0,
    sport: 0,
    reply: None,
});
static NEXT_ID: AtomicU16 = AtomicU16::new(0x2a17);

fn be16(b: &[u8], i: usize) -> u16 {
    ((b[i] as u16) << 8) | b[i + 1] as u16
}

fn checksum(data: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    let mut i = 0;
    while i + 1 < data.len() {
        sum += ((data[i] as u32) << 8) | data[i + 1] as u32;
        i += 2;
    }
    if i < data.len() {
        sum += (data[i] as u32) << 8;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !(sum as u16)
}

fn same_subnet(a: [u8; 4], b: [u8; 4], mask: [u8; 4]) -> bool {
    (0..4).all(|i| a[i] & mask[i] == b[i] & mask[i])
}

/// Called by net_thread for every IPv4 packet (Ethernet header already removed).
/// Returns true if it was the reply we are waiting for, so the caller skips it.
pub fn handle_udp(ip: &[u8]) -> bool {
    if ip.len() < 20 || ip[0] >> 4 != 4 {
        return false;
    }
    let ihl = ((ip[0] & 0x0F) as usize) * 4;
    if ihl < 20 || ip.len() < ihl + 8 || ip[9] != 17 {
        return false; // not UDP
    }
    // the Ethernet frame may be padded, so trust the IP total length
    let end = (be16(ip, 2) as usize).min(ip.len());
    if end < ihl + 8 {
        return false;
    }
    let sport = be16(ip, ihl);
    let dport = be16(ip, ihl + 2);
    let ulen = be16(ip, ihl + 4) as usize;
    if sport != DNS_PORT {
        return false;
    }
    let payload_end = (ihl + ulen).min(end);
    if payload_end < ihl + 8 + 12 {
        return false; // shorter than a DNS header
    }
    let payload = &ip[ihl + 8..payload_end];

    let mut pending = PENDING.lock();
    if dport != pending.sport || be16(payload, 0) != pending.id {
        return false; // not for the query we are waiting on
    }
    pending.reply = Some(payload.to_vec());
    true
}

/// Finds the MAC to send to: ARP cache first, otherwise ask for it.
fn resolve_mac(dev: &Arc<dyn NetDevice>, cfg: &Config, target: [u8; 4]) -> Result<MacAddr, &'static str> {
    let lookup = || {
        arp::entries()
            .into_iter()
            .find(|(ip, _)| *ip == target)
            .map(|(_, m)| m)
    };
    if let Some(m) = lookup() {
        return Ok(m);
    }

    let mac = dev.mac();
    let mut req = Vec::with_capacity(42);
    req.extend_from_slice(&[0xFF; 6]); // broadcast
    req.extend_from_slice(&mac);
    req.extend_from_slice(&[0x08, 0x06]); // ARP
    req.extend_from_slice(&[0x00, 0x01, 0x08, 0x00, 6, 4, 0x00, 0x01]); // ethernet, IPv4, request
    req.extend_from_slice(&mac);
    req.extend_from_slice(&cfg.ip);
    req.extend_from_slice(&[0; 6]);
    req.extend_from_slice(&target);

    for _ in 0..3 {
        dev.send(&req).map_err(|_| "cannot send ARP request")?;
        let start = interrupts::ticks();
        while interrupts::ticks() - start < 30 {
            if let Some(m) = lookup() {
                return Ok(m);
            }
            scheduler::sleep_ms(10);
        }
    }
    Err("no ARP reply from the next hop")
}

fn send_udp(
    dev: &Arc<dyn NetDevice>,
    cfg: &Config,
    dst_mac: MacAddr,
    dst_ip: [u8; 4],
    sport: u16,
    dport: u16,
    ip_id: u16,
    payload: &[u8],
) -> Result<(), &'static str> {
    let udp_len = 8 + payload.len();
    let total = 20 + udp_len;

    let mut ip = [0u8; 20];
    ip[0] = 0x45; // IPv4, 20-byte header
    ip[2..4].copy_from_slice(&(total as u16).to_be_bytes());
    ip[4..6].copy_from_slice(&ip_id.to_be_bytes());
    ip[6] = 0x40; // don't fragment
    ip[8] = 64; // TTL
    ip[9] = 17; // UDP
    ip[12..16].copy_from_slice(&cfg.ip);
    ip[16..20].copy_from_slice(&dst_ip);
    let sum = checksum(&ip);
    ip[10..12].copy_from_slice(&sum.to_be_bytes());

    let mut frame = Vec::with_capacity(14 + total);
    frame.extend_from_slice(&dst_mac);
    frame.extend_from_slice(&dev.mac());
    frame.extend_from_slice(&[0x08, 0x00]); // IPv4
    frame.extend_from_slice(&ip);
    frame.extend_from_slice(&sport.to_be_bytes());
    frame.extend_from_slice(&dport.to_be_bytes());
    frame.extend_from_slice(&(udp_len as u16).to_be_bytes());
    frame.extend_from_slice(&[0, 0]); // UDP checksum 0 = "not used", allowed over IPv4
    frame.extend_from_slice(payload);

    dev.send(&frame).map_err(|_| "cannot send packet")
}

fn encode_name(name: &str, out: &mut Vec<u8>) -> Result<(), &'static str> {
    let name = name.trim_end_matches('.');
    if name.is_empty() || name.len() > 253 || !name.is_ascii() {
        return Err("invalid host name");
    }
    for label in name.split('.') {
        let bytes = label.as_bytes();
        if bytes.is_empty() || bytes.len() > 63 {
            return Err("invalid host name");
        }
        out.push(bytes.len() as u8);
        out.extend_from_slice(bytes);
    }
    out.push(0);
    Ok(())
}

fn build_query(name: &str, id: u16) -> Result<Vec<u8>, &'static str> {
    let mut q = Vec::with_capacity(32 + name.len());
    q.extend_from_slice(&id.to_be_bytes());
    q.extend_from_slice(&[0x01, 0x00]); // standard query, recursion desired
    q.extend_from_slice(&[0, 1, 0, 0, 0, 0, 0, 0]); // 1 question
    encode_name(name, &mut q)?;
    q.extend_from_slice(&[0, 1, 0, 1]); // type A, class IN
    Ok(q)
}

/// Skips a (possibly compressed) name and returns the offset just after it.
fn skip_name(msg: &[u8], mut pos: usize) -> Result<usize, &'static str> {
    loop {
        let b = *msg.get(pos).ok_or("malformed DNS reply")?;
        if b == 0 {
            return Ok(pos + 1);
        }
        if b & 0xC0 == 0xC0 {
            return Ok(pos + 2); // compression pointer ends the name
        }
        pos += 1 + b as usize;
    }
}

fn parse_reply(msg: &[u8], id: u16) -> Result<[u8; 4], &'static str> {
    if msg.len() < 12 || be16(msg, 0) != id {
        return Err("malformed DNS reply");
    }
    let flags = be16(msg, 2);
    if flags & 0x8000 == 0 {
        return Err("malformed DNS reply");
    }
    match flags & 0x000F {
        0 => {}
        3 => return Err("no such host"),
        _ => return Err("DNS server error"),
    }

    let questions = be16(msg, 4) as usize;
    let answers = be16(msg, 6) as usize;
    let mut pos = 12;
    for _ in 0..questions {
        pos = skip_name(msg, pos)? + 4; // name + type + class
    }
    // A name like www.youtube.com usually comes back as a CNAME followed by the
    // A record, so walk every answer and take the first IPv4 address.
    for _ in 0..answers {
        pos = skip_name(msg, pos)?;
        if pos + 10 > msg.len() {
            return Err("malformed DNS reply");
        }
        let kind = be16(msg, pos);
        let class = be16(msg, pos + 2);
        let rdlen = be16(msg, pos + 8) as usize;
        pos += 10;
        if pos + rdlen > msg.len() {
            return Err("malformed DNS reply");
        }
        if kind == 1 && class == 1 && rdlen == 4 {
            return Ok([msg[pos], msg[pos + 1], msg[pos + 2], msg[pos + 3]]);
        }
        pos += rdlen;
    }
    Err("no IPv4 address for that name")
}

/// Looks up the IPv4 address of `name`. Blocks for up to about 3 seconds.
pub fn resolve(name: &str) -> Result<[u8; 4], &'static str> {
    let dev = interface().ok_or("no network interface")?;
    let cfg = config();

    // Same subnet: ask the DNS server directly. Otherwise go through the gateway.
    let hop = if same_subnet(cfg.ip, DNS_SERVER, cfg.netmask) {
        DNS_SERVER
    } else {
        cfg.gateway
    };
    let dst_mac = resolve_mac(&dev, &cfg, hop)?;

    for _ in 0..3 {
        let id = NEXT_ID.fetch_add(1, Relaxed) ^ (interrupts::ticks() as u16);
        let sport = 49152 + (id % 1000);
        let query = build_query(name, id)?;

        {
            let mut pending = PENDING.lock();
            pending.id = id;
            pending.sport = sport;
            pending.reply = None;
        }
        send_udp(&dev, &cfg, dst_mac, DNS_SERVER, sport, DNS_PORT, id, &query)?;

        let start = interrupts::ticks();
        while interrupts::ticks() - start < 100 {
            // up to one second
            let reply = PENDING.lock().reply.take();
            if let Some(msg) = reply {
                return parse_reply(&msg, id);
            }
            scheduler::sleep_ms(10);
        }
    }
    Err("DNS request timed out")
}
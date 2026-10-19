use super::{config, ethernet, ipv4::same_subnet};
use crate::base::error::{Error, Result};
use crate::base::sync::Mutex;
use crate::drivers::network::NetDevice;
use crate::scheduler;
use alloc::{sync::Arc, vec::Vec};

const CACHE_SIZE: usize = 16;

static CACHE: Mutex<Vec<([u8; 4], [u8; 6])>> = Mutex::new(Vec::new());

pub fn lookup(ip: [u8; 4]) -> Option<[u8; 6]> {
    CACHE.lock().iter().find(|e| e.0 == ip).map(|e| e.1)
}

pub fn entries() -> Vec<([u8; 4], [u8; 6])> {
    CACHE.lock().clone()
}

fn learn(ip: [u8; 4], mac: [u8; 6]) {
    if ip == [0; 4] {
        return;
    }
    let mut cache = CACHE.lock();
    if let Some(entry) = cache.iter_mut().find(|e| e.0 == ip) {
        entry.1 = mac;
        return;
    }
    if cache.len() >= CACHE_SIZE {
        cache.remove(0);
    }
    cache.push((ip, mac));
}

fn packet(op: u16, sha: [u8; 6], spa: [u8; 4], tha: [u8; 6], tpa: [u8; 4]) -> [u8; 28] {
    let mut p = [0u8; 28];
    p[0..2].copy_from_slice(&1u16.to_be_bytes()); // hardware type: Ethernet
    p[2..4].copy_from_slice(&0x0800u16.to_be_bytes()); // protocol: IPv4
    p[4] = 6;
    p[5] = 4;
    p[6..8].copy_from_slice(&op.to_be_bytes()); // 1 = request, 2 = reply
    p[8..14].copy_from_slice(&sha);
    p[14..18].copy_from_slice(&spa);
    p[18..24].copy_from_slice(&tha);
    p[24..28].copy_from_slice(&tpa);
    p
}

/// Called by the network thread for every ARP frame.
pub fn handle(dev: &Arc<dyn NetDevice>, p: &[u8]) {
    if p.len() < 28 || p[0..2] != [0, 1] || p[2..4] != [8, 0] || p[4] != 6 || p[5] != 4 {
        return;
    }
    let op = u16::from_be_bytes([p[6], p[7]]);
    let sha: [u8; 6] = p[8..14].try_into().unwrap();
    let spa: [u8; 4] = p[14..18].try_into().unwrap();
    let tpa: [u8; 4] = p[24..28].try_into().unwrap();

    learn(spa, sha);

    let cfg = config();
    if op == 1 && tpa == cfg.ip {
        let reply = packet(2, dev.mac(), cfg.ip, sha, spa);
        let _ = dev.send(&ethernet::build(sha, dev.mac(), ethernet::ARP, &reply));
    }
}

/// The MAC address to use for `ip`: the host itself on the local network, otherwise
/// the gateway. Asks the network and waits up to about a second.
pub fn resolve(dev: &Arc<dyn NetDevice>, ip: [u8; 4]) -> Result<[u8; 6]> {
    let cfg = config();
    let hop = if same_subnet(cfg.ip, ip, cfg.netmask) { ip } else { cfg.gateway };

    for _ in 0..5 {
        if let Some(mac) = lookup(hop) {
            return Ok(mac);
        }
        let request = packet(1, dev.mac(), cfg.ip, [0; 6], hop);
        dev.send(&ethernet::build(ethernet::BROADCAST, dev.mac(), ethernet::ARP, &request))?;
        for _ in 0..20 {
            scheduler::sleep_ms(10);
            if let Some(mac) = lookup(hop) {
                return Ok(mac);
            }
        }
    }
    Err(Error::Unreachable)
}
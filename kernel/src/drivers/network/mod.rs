#![allow(dead_code)]
use crate::base::sync::Mutex;
use alloc::{sync::Arc, vec::Vec};
use errors::Result;
pub mod e1000;
pub mod wifi;
pub mod arp;
pub mod ethernet;
pub mod icmp;
pub mod ipv4;

pub type MacAddr = [u8; 6];

#[derive(Clone, Copy, Default)]
pub struct NetStats {
    pub rx_packets: u64,
    pub tx_packets: u64,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
}

/// A network interface that carries Ethernet frames.
pub trait NetDevice: Send + Sync {
    fn name(&self) -> &str;
    fn mac(&self) -> MacAddr;
    fn link_up(&self) -> bool;

    /// Sends one complete Ethernet frame (14-byte header + payload, no CRC).
    fn send(&self, frame: &[u8]) -> Result<()>;

    /// Non-blocking. Some(n) = a frame of n bytes was copied into `buf` (n = 0 means one
    /// was received and dropped), None = nothing waiting.
    fn receive(&self, buf: &mut [u8]) -> Result<Option<usize>>;

    fn stats(&self) -> NetStats {
        NetStats::default()
    }
}

static DEVICES: Mutex<Vec<Arc<dyn NetDevice>>> = Mutex::new(Vec::new());

pub fn register(device: Arc<dyn NetDevice>) {
    DEVICES.lock().push(device);
}

pub fn get(name: &str) -> Option<Arc<dyn NetDevice>> {
    DEVICES.lock().iter().find(|d| d.name() == name).cloned()
}

pub fn list() -> Vec<Arc<dyn NetDevice>> {
    DEVICES.lock().clone()
}

use crate::drivers::network::{self as nic};
use crate::{log_warn, process, scheduler};

#[derive(Clone, Copy)]
pub struct Config {
    pub ip: [u8; 4],
    pub netmask: [u8; 4],
    pub gateway: [u8; 4],
}

/// QEMU's built-in "user" network: the guest is 10.0.2.15, the gateway 10.0.2.2.
/// A DHCP client replaces this once UDP exists.
pub static CONFIG: Mutex<Config> = Mutex::new(Config {
    ip: [10, 0, 2, 15],
    netmask: [255, 255, 255, 0],
    gateway: [10, 0, 2, 2],
});

pub fn config() -> Config {
    *CONFIG.lock()
}

pub fn set_config(config: Config) {
    *CONFIG.lock() = config;
}

/// The interface the stack uses: wired first, then WiFi.
pub fn interface() -> Option<Arc<dyn NetDevice>> {
    nic::get("eth0").or_else(|| nic::get("wlan0"))
}

pub fn start() {
    if interface().is_none() {
        return;
    }
    if let Err(e) = process::spawn("net", net_thread) {
        log_warn!("network: cannot start the network thread: {}", e);
    }
}

/// Receives frames and hands them to ARP / IPv4. Polls every 10 ms, and keeps going
/// without sleeping while frames are arriving.
fn net_thread() {
    process::detach(scheduler::current_pid());
    let Some(dev) = interface() else { return };
    let mut buf = [0u8; 2048];
    loop {
        let mut busy = false;
        while let Ok(Some(n)) = dev.receive(&mut buf) {
            busy = true;
            if n >= 14 {
                handle_frame(&dev, &buf[..n]);
            }
        }
        if busy {
            scheduler::yield_now();
        } else {
            scheduler::sleep_ms(10);
        }
    }
}

fn handle_frame(dev: &Arc<dyn NetDevice>, frame: &[u8]) {
    let Some(eth) = ethernet::parse(frame) else {
        return;
    };
    if eth.dst != dev.mac() && eth.dst != ethernet::BROADCAST {
        return;
    }
    match eth.ethertype {
        ethernet::ARP => arp::handle(dev, eth.payload),
        ethernet::IPV4 => ipv4::handle(dev, eth.src, eth.payload),
        _ => {}
    }
}

use crate::drivers::network::{self as nic, wifi};
use crate::drivers::network::{self, arp, icmp, Config};
use crate::security::wpa2;
use crate::{interrupts, scheduler};
use crate::kprintln;
use alloc::{format, string::String};

pub fn run(cmd: &str, args: &[&str]) {
    match cmd {
        "ifconfig" => ifconfig(args),
        "ping" => ping(args),
        "arp" => arp_table(),
        "wifi" => wifi_cmd(args),
        _ => {}
    }
}

fn parse_ip(text: &str) -> Option<[u8; 4]> {
    let mut ip = [0u8; 4];
    let mut parts = text.split('.');
    for byte in ip.iter_mut() {
        *byte = parts.next()?.parse().ok()?;
    }
    if parts.next().is_some() {
        return None;
    }
    Some(ip)
}

fn ip_str(ip: [u8; 4]) -> String {
    format!("{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3])
}

fn ifconfig(args: &[&str]) {
    if args.first() == Some(&"set") {
        if args.len() != 4 {
            kprintln!("usage: ifconfig set <ip> <netmask> <gateway>");
            return;
        }
        match (parse_ip(args[1]), parse_ip(args[2]), parse_ip(args[3])) {
            (Some(ip), Some(netmask), Some(gateway)) => {
                network::set_config(Config { ip, netmask, gateway });
                kprintln!("address set");
            }
            _ => kprintln!("ifconfig: invalid address"),
        }
        return;
    }

    let devices = nic::list();
    if devices.is_empty() {
        kprintln!("no network interfaces");
        return;
    }
    let active = network::interface().map(|d| String::from(d.name()));
    let cfg = network::config();
    for dev in devices {
        let m = dev.mac();
        kprintln!(
            "{}  link {}  mac {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            dev.name(), if dev.link_up() { "up" } else { "down" }, m[0], m[1], m[2], m[3], m[4], m[5]
        );
        if active.as_deref() == Some(dev.name()) {
            kprintln!(
                "      inet {}  netmask {}  gateway {}",
                ip_str(cfg.ip), ip_str(cfg.netmask), ip_str(cfg.gateway)
            );
        }
        let s = dev.stats();
        kprintln!(
            "      rx {} packets ({} bytes)   tx {} packets ({} bytes)",
            s.rx_packets, s.rx_bytes, s.tx_packets, s.tx_bytes
        );
    }
}

fn arp_table() {
    let entries = arp::entries();
    if entries.is_empty() {
        kprintln!("the ARP table is empty");
    }
    for (ip, m) in entries {
        kprintln!(
            "{:<16} {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            ip_str(ip), m[0], m[1], m[2], m[3], m[4], m[5]
        );
    }
}

fn ping(args: &[&str]) {
    let Some(target) = args.first().and_then(|a| parse_ip(a)) else {
        kprintln!("usage: ping <a.b.c.d>");
        return;
    };
    let Some(dev) = network::interface() else {
        kprintln!("ping: no network interface");
        return;
    };

    kprintln!("PING {}", ip_str(target));
    let (mut sent, mut received) = (0, 0);
    for _ in 0..4 {
        let seq = icmp::next_seq();
        let start = interrupts::ticks();
        sent += 1;

        match icmp::send_echo(&dev, target, seq) {
            Err(e) => kprintln!("ping: {}", e),
            Ok(()) => {
                let mut reply = None;
                while interrupts::ticks() - start < 100 {
                    // up to one second
                    if let Some(tick) = icmp::reply_tick(seq) {
                        reply = Some(tick);
                        break;
                    }
                    scheduler::sleep_ms(10);
                }
                match reply {
                    Some(tick) => {
                        received += 1;
                        kprintln!("reply from {}: seq={} time={} ms", ip_str(target), seq, (tick - start) * 10);
                    }
                    None => kprintln!("request timed out (seq={})", seq),
                }
            }
        }

        let elapsed = interrupts::ticks() - start;
        if elapsed < 100 {
            scheduler::sleep_ms((100 - elapsed) * 10); // one request per second
        }
    }
    kprintln!("{} sent, {} received", sent, received);
}

fn wifi_cmd(args: &[&str]) {
    match args.first().copied() {
        Some("selftest") => {
            if wpa2::selftest() {
                kprintln!("wpa2: PMK derivation matches the IEEE 802.11i test vector: PASS");
            } else {
                kprintln!("wpa2: PMK derivation does NOT match the test vector: FAIL");
            }
        }
        Some("scan") => {
            let adapters = wifi::list_wifi();
            if adapters.is_empty() {
                no_wifi();
            }
            for adapter in adapters {
                match adapter.scan() {
                    Ok(networks) => {
                        for n in networks {
                            kprintln!(
                                "{:<32} ch {:>3}  {:>4} dBm  {}",
                                n.ssid, n.channel, n.signal_dbm, n.security.name()
                            );
                        }
                    }
                    Err(e) => kprintln!("wifi: {}: scan failed: {}", adapter.name(), e),
                }
            }
        }
        Some("connect") => {
            let (Some(&ssid), passphrase) = (args.get(1), args.get(2)) else {
                kprintln!("usage: wifi connect <ssid> [passphrase]");
                return;
            };
            let pmk = match passphrase {
                Some(p) => match wpa2::derive_pmk(p, ssid) {
                    Ok(key) => Some(key),
                    Err(e) => {
                        kprintln!("wifi: {}", e);
                        return;
                    }
                },
                None => None,
            };
            match wifi::list_wifi().first() {
                None => no_wifi(),
                Some(adapter) => match adapter.connect(ssid, pmk.as_ref()) {
                    Ok(()) => kprintln!("connecting to {}...", ssid),
                    Err(e) => kprintln!("wifi: {}", e),
                },
            }
        }
        _ => {
            let adapters = wifi::list_wifi();
            if adapters.is_empty() {
                no_wifi();
            }
            for adapter in adapters {
                kprintln!("{}: {}", adapter.name(), match adapter.state() {
                    wifi::LinkState::Disconnected => "disconnected",
                    wifi::LinkState::Scanning => "scanning",
                    wifi::LinkState::Connecting => "connecting",
                    wifi::LinkState::Connected => "connected",
                });
            }
            kprintln!("usage: wifi [scan | connect <ssid> [passphrase] | selftest]");
        }
    }
}

fn no_wifi() {
    kprintln!("no WiFi adapter found (QEMU does not emulate WiFi hardware)");
}
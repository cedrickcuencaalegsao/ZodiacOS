// #![allow(dead_code)]
use super::NetDevice;
use crate::base::sync::Mutex;
use alloc::{string::String, sync::Arc, vec::Vec};
use errors::Result;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Security {
    Open,
    Wep,
    Wpa2Psk,
    Wpa3Sae,
}

impl Security {
    pub fn name(self) -> &'static str {
        match self {
            Security::Open => "open",
            Security::Wep => "WEP",
            Security::Wpa2Psk => "WPA2-PSK",
            Security::Wpa3Sae => "WPA3-SAE",
        }
    }
}

pub struct ScanResult {
    pub ssid: String,
    pub bssid: [u8; 6],
    pub channel: u8,
    pub signal_dbm: i8,
    pub security: Security,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LinkState {
    Disconnected,
    Scanning,
    Connecting,
    Connected,
}

/// A WiFi adapter. A driver implements this and also registers a `NetDevice` for
/// the data path (see `net()`).
pub trait WifiDevice: Send + Sync {
    fn name(&self) -> &str;
    fn state(&self) -> LinkState;
    fn scan(&self) -> Result<Vec<ScanResult>>;

    /// Joins a network. `pmk` is the 32-byte pairwise master key from
    /// `security::wpa2::derive_pmk`, or None for an open network. What happens next
    /// depends on the chip: firmware-based chips run the WPA2 handshake themselves
    /// from the key, others need a handshake implementation on the host.
    fn connect(&self, ssid: &str, pmk: Option<&[u8; 32]>) -> Result<()>;
    fn disconnect(&self) -> Result<()>;

    /// The data path once connected: it carries Ethernet frames like any NIC.
    fn net(&self) -> Arc<dyn NetDevice>;
}

static WIFI: Mutex<Vec<Arc<dyn WifiDevice>>> = Mutex::new(Vec::new());

#[allow(dead_code)]
pub fn register_wifi(device: Arc<dyn WifiDevice>) {
    WIFI.lock().push(device);
}

pub fn list_wifi() -> Vec<Arc<dyn WifiDevice>> {
    WIFI.lock().clone()
}

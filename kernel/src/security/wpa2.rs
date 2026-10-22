#![allow(dead_code)]
use errors::Error;
use hmac::{Hmac, Mac};
use pbkdf2::pbkdf2_hmac;
use sha1::Sha1;

/// The pairwise master key of a WPA2-Personal network:
/// PBKDF2-HMAC-SHA1(passphrase, SSID, 4096 rounds) -> 256 bits.
pub fn derive_pmk(passphrase: &str, ssid: &str) -> Result<[u8; 32], Error> {
    if !(8..=63).contains(&passphrase.len()) || !passphrase.is_ascii() {
        return Err(Error::InvalidArgument); // WPA2 passphrases are 8 to 63 ASCII characters
    }
    if ssid.is_empty() || ssid.len() > 32 {
        return Err(Error::InvalidArgument);
    }
    let mut pmk = [0u8; 32];
    pbkdf2_hmac::<Sha1>(passphrase.as_bytes(), ssid.as_bytes(), 4096, &mut pmk);
    Ok(pmk)
}

/// The pairwise transient key from the 4-way handshake: PRF-384 over both MAC
/// addresses and both nonces. Layout: KCK (bytes 0-15, signs handshake messages),
/// KEK (16-31, encrypts the group key), TK (32-47, encrypts data with CCMP).
pub fn derive_ptk(
    pmk: &[u8; 32],
    ap_mac: &[u8; 6],
    client_mac: &[u8; 6],
    anonce: &[u8; 32],
    snonce: &[u8; 32],
) -> [u8; 48] {
    let (mac_a, mac_b) = if ap_mac <= client_mac {
        (ap_mac, client_mac)
    } else {
        (client_mac, ap_mac)
    };
    let (nonce_a, nonce_b) = if anonce <= snonce {
        (anonce, snonce)
    } else {
        (snonce, anonce)
    };

    let mut data = [0u8; 76];
    data[0..6].copy_from_slice(mac_a);
    data[6..12].copy_from_slice(mac_b);
    data[12..44].copy_from_slice(nonce_a);
    data[44..76].copy_from_slice(nonce_b);

    // PRF: HMAC-SHA1(PMK, label || 0x00 || data || counter), counter = 0, 1, 2
    let mut out = [0u8; 60];
    for i in 0..3u8 {
        let mut mac = Hmac::<Sha1>::new_from_slice(pmk).expect("HMAC accepts any key length");
        mac.update(b"Pairwise key expansion");
        mac.update(&[0]);
        mac.update(&data);
        mac.update(&[i]);
        let block = mac.finalize().into_bytes();
        let start = i as usize * 20;
        out[start..start + 20].copy_from_slice(&block);
    }
    let mut ptk = [0u8; 48];
    ptk.copy_from_slice(&out[..48]);
    ptk
}

/// Checks `derive_pmk` against the official IEEE 802.11i test vector
/// (passphrase "password", SSID "IEEE").
pub fn selftest() -> bool {
    const EXPECTED: [u8; 32] = [
        0xf4, 0x2c, 0x6f, 0xc5, 0x2d, 0xf0, 0xeb, 0xef, 0x9e, 0xbb, 0x4b, 0x90, 0xb3, 0x8a, 0x5f,
        0x90, 0x2e, 0x83, 0xfe, 0x1b, 0x13, 0x5a, 0x70, 0xe2, 0x3a, 0xed, 0x76, 0x2e, 0x97, 0x10,
        0xa1, 0x2e,
    ];
    derive_pmk("password", "IEEE")
        .map(|pmk| pmk == EXPECTED)
        .unwrap_or(false)
}

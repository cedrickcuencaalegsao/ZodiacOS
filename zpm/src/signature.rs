use std::collections::BTreeMap;

use ed25519_dalek::{Signature, VerifyingKey};

use crate::error::{Result, ZpmError};

/// The publisher keys this installation trusts. A signature only proves that
/// the holder of a trusted key approved the manifest; the manifest in turn
/// pins the archive's SHA-256, which closes the chain down to the bytes.
pub struct TrustStore {
    keys: BTreeMap<String, VerifyingKey>,
}

impl TrustStore {
    pub fn from_hex_map(map: &BTreeMap<String, String>) -> Result<Self> {
        let mut keys = BTreeMap::new();
        for (id, hex_key) in map {
            let bytes = hex::decode(hex_key.trim())
                .map_err(|e| ZpmError::Config(format!("trusted key '{id}' is not valid hex: {e}")))?;
            let arr: [u8; 32] = bytes
                .try_into()
                .map_err(|_| ZpmError::Config(format!("trusted key '{id}' must be 32 bytes (64 hex characters)")))?;
            let key = VerifyingKey::from_bytes(&arr)
                .map_err(|e| ZpmError::Config(format!("trusted key '{id}' is not a valid ed25519 key: {e}")))?;
            keys.insert(id.clone(), key);
        }
        Ok(Self { keys })
    }

    pub fn verify(&self, key_id: &str, message: &[u8], signature_hex: &str) -> Result<()> {
        let key = self
            .keys
            .get(key_id)
            .ok_or_else(|| ZpmError::Signature(format!("key '{key_id}' is not in the trusted key list")))?;
        let bytes = hex::decode(signature_hex.trim())
            .map_err(|_| ZpmError::Signature("signature is not valid hex".into()))?;
        let sig = Signature::from_slice(&bytes).map_err(|_| ZpmError::Signature("signature has the wrong length".into()))?;
        key.verify_strict(message, &sig)
            .map_err(|_| ZpmError::Signature(format!("signature does not match key '{key_id}'")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    fn store(seed: u8) -> (SigningKey, TrustStore) {
        let signing = SigningKey::from_bytes(&[seed; 32]);
        let mut map = BTreeMap::new();
        map.insert("official".to_string(), hex::encode(signing.verifying_key().to_bytes()));
        (signing, TrustStore::from_hex_map(&map).unwrap())
    }

    #[test]
    fn accepts_valid_signature() {
        let (key, trust) = store(7);
        let sig = hex::encode(key.sign(b"manifest").to_bytes());
        assert!(trust.verify("official", b"manifest", &sig).is_ok());
    }

    #[test]
    fn rejects_tampered_message_wrong_key_and_unknown_id() {
        let (key, trust) = store(7);
        let sig = hex::encode(key.sign(b"manifest").to_bytes());
        assert!(trust.verify("official", b"manifest!", &sig).is_err());
        assert!(trust.verify("someone-else", b"manifest", &sig).is_err());

        let other = SigningKey::from_bytes(&[9; 32]);
        let forged = hex::encode(other.sign(b"manifest").to_bytes());
        assert!(trust.verify("official", b"manifest", &forged).is_err());
        assert!(trust.verify("official", b"manifest", "zz").is_err());
        assert!(trust.verify("official", b"manifest", "abcd").is_err());
    }

    #[test]
    fn rejects_bad_key_config() {
        let mut map = BTreeMap::new();
        map.insert("k".to_string(), "abcd".to_string());
        assert!(TrustStore::from_hex_map(&map).is_err());
    }
}
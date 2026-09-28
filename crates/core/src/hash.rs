//! sha256 hashing helpers. Hashes print and serialize as 64 lowercase hex characters.

use std::fmt;
use std::str::FromStr;

use serde::{de::Error as _, Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};

/// A 32 byte sha256 digest.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Hash([u8; 32]);

/// Error returned when a string is not a 64 character lowercase hex hash.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum HashError {
    /// The string does not have exactly 64 characters.
    #[error("hash must be 64 hex characters, got {0}")]
    BadLength(usize),
    /// The string contains something that is not a hex digit.
    #[error("hash is not valid hex: {0}")]
    BadHex(String),
}

impl Hash {
    /// The all zero hash, used as the parent of the genesis block.
    pub const ZERO: Hash = Hash([0u8; 32]);

    /// Wraps 32 raw bytes.
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Hash(bytes)
    }

    /// The raw 32 bytes.
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// The raw 32 bytes, by value.
    pub const fn to_bytes(self) -> [u8; 32] {
        self.0
    }

    /// The 64 character lowercase hex form.
    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    /// Parses a 64 character hex string (upper or lower case).
    pub fn from_hex(s: &str) -> Result<Self, HashError> {
        if s.len() != 64 {
            return Err(HashError::BadLength(s.len()));
        }
        let bytes = hex::decode(s).map_err(|e| HashError::BadHex(e.to_string()))?;
        let mut out = [0u8; 32];
        out.copy_from_slice(&bytes);
        Ok(Hash(out))
    }
}

/// Hashes a byte slice with sha256.
pub fn sha256(data: &[u8]) -> Hash {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let digest = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    Hash(out)
}

/// Hashes a sequence of byte slices as if they were concatenated.
pub fn sha256_parts(parts: &[&[u8]]) -> Hash {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part);
    }
    let digest = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    Hash(out)
}

impl fmt::Display for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl fmt::Debug for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Hash({})", self.to_hex())
    }
}

impl FromStr for Hash {
    type Err = HashError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Hash::from_hex(s)
    }
}

impl Serialize for Hash {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for Hash {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Hash::from_hex(&s).map_err(D::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vector_empty_string() {
        assert_eq!(
            sha256(b"").to_hex(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn known_vector_abc() {
        assert_eq!(
            sha256(b"abc").to_hex(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn parts_equal_concatenation() {
        assert_eq!(sha256_parts(&[b"bot", b"chain"]), sha256(b"botchain"));
    }

    #[test]
    fn hex_roundtrip() {
        let h = sha256(b"botchain");
        let parsed: Hash = h.to_hex().parse().expect("valid hex");
        assert_eq!(h, parsed);
        assert_eq!(h.to_hex().len(), 64);
        assert!(h.to_hex().chars().all(|c| !c.is_ascii_uppercase()));
    }

    #[test]
    fn zero_hash_is_64_zeros() {
        assert_eq!(Hash::ZERO.to_hex(), "0".repeat(64));
    }

    #[test]
    fn bad_hash_strings() {
        assert_eq!(Hash::from_hex("abc"), Err(HashError::BadLength(3)));
        let bad = "z".repeat(64);
        assert!(matches!(Hash::from_hex(&bad), Err(HashError::BadHex(_))));
    }

    #[test]
    fn serde_is_a_hex_string() {
        let h = sha256(b"abc");
        let json = serde_json::to_string(&h).expect("serialize");
        assert_eq!(
            json,
            "\"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad\""
        );
        let back: Hash = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, h);
        assert!(serde_json::from_str::<Hash>("\"nope\"").is_err());
    }
}

//! Keys, addresses and signatures.
//!
//! An address is the base58 form of a 32 byte ed25519 public key, exactly the
//! encoding Solana uses, so an ordinary Solana wallet key is a botchain key.
//! A signature is the base58 form of the 64 byte ed25519 signature.

use std::fmt;
use std::str::FromStr;

use ed25519_dalek::{Signature as DalekSignature, Signer, SigningKey, VerifyingKey};
use serde::{de::Error as _, Deserialize, Deserializer, Serialize, Serializer};

/// Length of a public key, and therefore of an address, in bytes.
pub const ADDRESS_LEN: usize = 32;
/// Length of an ed25519 signature in bytes.
pub const SIGNATURE_LEN: usize = 64;
/// Length of an ed25519 secret key seed in bytes.
pub const SECRET_KEY_LEN: usize = 32;

/// Anything that can go wrong while parsing keys or checking signatures.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CryptoError {
    /// The string was empty.
    #[error("{what} is empty")]
    Empty {
        /// What was being parsed, for example "address".
        what: &'static str,
    },
    /// The string is not valid base58.
    #[error("{what} is not valid base58: {reason}")]
    BadBase58 {
        /// What was being parsed.
        what: &'static str,
        /// Why base58 decoding failed.
        reason: String,
    },
    /// The decoded bytes have the wrong length.
    #[error("{what} must decode to {expected} bytes, got {got}")]
    BadLength {
        /// What was being parsed.
        what: &'static str,
        /// The expected byte length.
        expected: usize,
        /// The length that was decoded.
        got: usize,
    },
    /// The bytes are not a valid ed25519 point.
    #[error("not a valid ed25519 public key: {0}")]
    BadPublicKey(String),
    /// The signature does not match the message and the address.
    #[error("signature does not verify")]
    BadSignature,
}

fn decode_base58(what: &'static str, s: &str, expected: usize) -> Result<Vec<u8>, CryptoError> {
    if s.is_empty() {
        return Err(CryptoError::Empty { what });
    }
    let bytes = bs58::decode(s)
        .into_vec()
        .map_err(|e| CryptoError::BadBase58 {
            what,
            reason: e.to_string(),
        })?;
    if bytes.len() != expected {
        return Err(CryptoError::BadLength {
            what,
            expected,
            got: bytes.len(),
        });
    }
    Ok(bytes)
}

/// A botchain address: a 32 byte ed25519 public key.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Address([u8; ADDRESS_LEN]);

impl Address {
    /// Wraps 32 raw public key bytes without checking that they are a point.
    pub const fn from_bytes(bytes: [u8; ADDRESS_LEN]) -> Self {
        Address(bytes)
    }

    /// The raw 32 bytes.
    pub const fn as_bytes(&self) -> &[u8; ADDRESS_LEN] {
        &self.0
    }

    /// The raw 32 bytes, by value.
    pub const fn to_bytes(self) -> [u8; ADDRESS_LEN] {
        self.0
    }

    /// The base58 text form.
    pub fn to_base58(&self) -> String {
        bs58::encode(self.0).into_string()
    }

    /// Parses a base58 address, with a clear error for bad input.
    pub fn parse(s: &str) -> Result<Self, CryptoError> {
        let bytes = decode_base58("address", s, ADDRESS_LEN)?;
        let mut out = [0u8; ADDRESS_LEN];
        out.copy_from_slice(&bytes);
        Ok(Address(out))
    }

    /// The ed25519 verifying key, if these bytes are a valid curve point.
    pub fn verifying_key(&self) -> Result<VerifyingKey, CryptoError> {
        VerifyingKey::from_bytes(&self.0).map_err(|e| CryptoError::BadPublicKey(e.to_string()))
    }

    /// Checks a signature over `message` made by this address.
    pub fn verify(&self, message: &[u8], signature: &Signature) -> Result<(), CryptoError> {
        let key = self.verifying_key()?;
        key.verify_strict(message, &signature.to_dalek())
            .map_err(|_| CryptoError::BadSignature)
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_base58())
    }
}

impl fmt::Debug for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Address({})", self.to_base58())
    }
}

impl FromStr for Address {
    type Err = CryptoError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Address::parse(s)
    }
}

impl From<VerifyingKey> for Address {
    fn from(key: VerifyingKey) -> Self {
        Address(key.to_bytes())
    }
}

impl Serialize for Address {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_base58())
    }
}

impl<'de> Deserialize<'de> for Address {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Address::parse(&s).map_err(D::Error::custom)
    }
}

/// A 64 byte ed25519 signature, base58 encoded in JSON.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Signature([u8; SIGNATURE_LEN]);

impl Signature {
    /// Wraps 64 raw signature bytes.
    pub const fn from_bytes(bytes: [u8; SIGNATURE_LEN]) -> Self {
        Signature(bytes)
    }

    /// The raw 64 bytes.
    pub const fn as_bytes(&self) -> &[u8; SIGNATURE_LEN] {
        &self.0
    }

    /// The raw 64 bytes, by value.
    pub const fn to_bytes(self) -> [u8; SIGNATURE_LEN] {
        self.0
    }

    /// The base58 text form, which is also the transaction id.
    pub fn to_base58(&self) -> String {
        bs58::encode(self.0).into_string()
    }

    /// Parses a base58 signature, with a clear error for bad input.
    pub fn parse(s: &str) -> Result<Self, CryptoError> {
        let bytes = decode_base58("signature", s, SIGNATURE_LEN)?;
        let mut out = [0u8; SIGNATURE_LEN];
        out.copy_from_slice(&bytes);
        Ok(Signature(out))
    }

    fn to_dalek(self) -> DalekSignature {
        DalekSignature::from_bytes(&self.0)
    }
}

impl fmt::Display for Signature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_base58())
    }
}

impl fmt::Debug for Signature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Signature({})", self.to_base58())
    }
}

impl FromStr for Signature {
    type Err = CryptoError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Signature::parse(s)
    }
}

impl Serialize for Signature {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_base58())
    }
}

impl<'de> Deserialize<'de> for Signature {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Signature::parse(&s).map_err(D::Error::custom)
    }
}

/// An ed25519 key pair: a secret seed plus the public key it derives.
pub struct Keypair(SigningKey);

impl Keypair {
    /// Generates a fresh random key pair from the OS random source.
    pub fn generate() -> Self {
        let mut rng = rand::rngs::OsRng;
        Keypair(SigningKey::generate(&mut rng))
    }

    /// Rebuilds a key pair from its 32 byte secret seed.
    pub fn from_secret_bytes(secret: [u8; SECRET_KEY_LEN]) -> Self {
        Keypair(SigningKey::from_bytes(&secret))
    }

    /// The 32 byte secret seed. Keep it secret.
    pub fn secret_bytes(&self) -> [u8; SECRET_KEY_LEN] {
        self.0.to_bytes()
    }

    /// The public key of this key pair, as an address.
    pub fn address(&self) -> Address {
        Address(self.0.verifying_key().to_bytes())
    }

    /// Signs an arbitrary message.
    pub fn sign(&self, message: &[u8]) -> Signature {
        Signature(self.0.sign(message).to_bytes())
    }
}

impl fmt::Debug for Keypair {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Never print the secret.
        write!(f, "Keypair({})", self.address())
    }
}

/// Checks a signature over `message` made by `address`.
pub fn verify(address: &Address, message: &[u8], signature: &Signature) -> Result<(), CryptoError> {
    address.verify(message, signature)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 8032 section 7.1, test 1.
    const RFC8032_SECRET: &str = "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60";
    const RFC8032_PUBLIC: &str = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";

    fn rfc8032_keypair() -> Keypair {
        let bytes = hex::decode(RFC8032_SECRET).expect("hex");
        let mut seed = [0u8; 32];
        seed.copy_from_slice(&bytes);
        Keypair::from_secret_bytes(seed)
    }

    #[test]
    fn fixed_secret_key_gives_known_public_key() {
        let kp = rfc8032_keypair();
        assert_eq!(hex::encode(kp.address().to_bytes()), RFC8032_PUBLIC);
        assert_eq!(
            kp.secret_bytes().to_vec(),
            hex::decode(RFC8032_SECRET).expect("hex")
        );
    }

    #[test]
    fn known_vector_signature_verifies_and_flipped_byte_fails() {
        let kp = rfc8032_keypair();
        let message = b"botchain:transfer:botchain-devnet:a:b:1:0";
        let sig = kp.sign(message);
        // Signing is deterministic in ed25519.
        assert_eq!(kp.sign(message), sig);
        assert_eq!(kp.address().verify(message, &sig), Ok(()));

        // Flip one bit of the signature: it must not verify.
        let mut broken = sig.to_bytes();
        broken[0] ^= 0x01;
        let broken = Signature::from_bytes(broken);
        assert_eq!(
            kp.address().verify(message, &broken),
            Err(CryptoError::BadSignature)
        );

        // Flip one byte of the message: it must not verify either.
        let mut other = message.to_vec();
        other[0] ^= 0x01;
        assert_eq!(
            kp.address().verify(&other, &sig),
            Err(CryptoError::BadSignature)
        );

        // Another key must not verify this signature.
        let stranger = Keypair::generate();
        assert_eq!(
            stranger.address().verify(message, &sig),
            Err(CryptoError::BadSignature)
        );
    }

    #[test]
    fn empty_message_signature_matches_rfc8032() {
        let kp = rfc8032_keypair();
        let sig = kp.sign(b"");
        assert_eq!(
            hex::encode(sig.to_bytes()),
            "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b"
        );
        assert_eq!(kp.address().verify(b"", &sig), Ok(()));
    }

    #[test]
    fn address_base58_is_solana_style() {
        // 32 zero bytes are the Solana system program address.
        let zero = Address::from_bytes([0u8; 32]);
        assert_eq!(zero.to_base58(), "1".repeat(32));
        let parsed = Address::parse(&zero.to_base58()).expect("parse");
        assert_eq!(parsed, zero);

        let kp = rfc8032_keypair();
        let text = kp.address().to_base58();
        assert!(text.len() >= 32 && text.len() <= 44, "got {text}");
        assert_eq!(Address::parse(&text).expect("parse"), kp.address());
    }

    #[test]
    fn bad_addresses_have_clear_errors() {
        assert_eq!(
            Address::parse(""),
            Err(CryptoError::Empty { what: "address" })
        );
        assert!(matches!(
            Address::parse("0OIl"),
            Err(CryptoError::BadBase58 { .. })
        ));
        // Valid base58, wrong length.
        let short = bs58::encode([1u8; 16]).into_string();
        assert_eq!(
            Address::parse(&short),
            Err(CryptoError::BadLength {
                what: "address",
                expected: 32,
                got: 16
            })
        );
    }

    #[test]
    fn bad_signatures_have_clear_errors() {
        assert_eq!(
            Signature::parse(""),
            Err(CryptoError::Empty { what: "signature" })
        );
        let short = bs58::encode([7u8; 10]).into_string();
        assert_eq!(
            Signature::parse(&short),
            Err(CryptoError::BadLength {
                what: "signature",
                expected: 64,
                got: 10
            })
        );
        assert!(matches!(
            Signature::parse("not base58 !"),
            Err(CryptoError::BadBase58 { .. })
        ));
    }

    #[test]
    fn signature_roundtrips_through_text() {
        let kp = Keypair::generate();
        let sig = kp.sign(b"hello");
        let text = sig.to_base58();
        assert_eq!(Signature::parse(&text).expect("parse"), sig);
        assert_eq!(text.parse::<Signature>().expect("parse"), sig);
    }

    #[test]
    fn generated_keys_are_distinct_and_usable() {
        let a = Keypair::generate();
        let b = Keypair::generate();
        assert_ne!(a.address(), b.address());
        let msg = b"arbitrary message";
        assert_eq!(verify(&a.address(), msg, &a.sign(msg)), Ok(()));
        // A key pair rebuilt from its seed behaves the same.
        let again = Keypair::from_secret_bytes(a.secret_bytes());
        assert_eq!(again.address(), a.address());
        assert_eq!(again.sign(msg), a.sign(msg));
    }

    #[test]
    fn serde_uses_base58_strings() {
        let kp = rfc8032_keypair();
        let json = serde_json::to_string(&kp.address()).expect("serialize");
        assert_eq!(json, format!("\"{}\"", kp.address().to_base58()));
        let back: Address = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, kp.address());
        assert!(serde_json::from_str::<Address>("\"oops\"").is_err());

        let sig = kp.sign(b"x");
        let json = serde_json::to_string(&sig).expect("serialize");
        let back: Signature = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, sig);
    }

    #[test]
    fn debug_does_not_leak_the_secret() {
        let kp = rfc8032_keypair();
        let text = format!("{kp:?}");
        assert!(text.contains(&kp.address().to_base58()));
        assert!(!text.contains(RFC8032_SECRET));
    }
}

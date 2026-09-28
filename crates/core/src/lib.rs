//! Core types of the botchain: addresses, keys, signatures, hashes and amounts.
//!
//! Everything here is chain independent plumbing: the pieces that the node,
//! the state machine and the RPC layer build on.

pub mod amount;
pub mod crypto;
pub mod hash;

pub use amount::{Amount, AmountError};
pub use crypto::{verify, Address, CryptoError, Keypair, Signature};
pub use hash::{sha256, sha256_parts, Hash, HashError};

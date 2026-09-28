//! Core types of the botchain: addresses, keys, signatures, hashes and amounts.
//!
//! Everything here is chain independent plumbing: the pieces that the node,
//! the state machine and the RPC layer build on.

pub mod account;
pub mod amount;
pub mod crypto;
pub mod genesis;
pub mod hash;
pub mod state;
pub mod tx;

pub use account::{Account, AccountView};
pub use amount::{Amount, AmountError};
pub use crypto::{verify, Address, CryptoError, Keypair, Signature};
pub use genesis::{Allocation, Genesis, GenesisError};
pub use hash::{sha256, sha256_parts, Hash, HashError};
pub use state::{State, StateError};
pub use tx::{transfer_message, Transfer, TxError};

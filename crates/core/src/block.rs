//! Blocks: the unit the sequencer seals on every tick.
//!
//! A block commits to its height, its parent, the state root *after* the
//! block, its timestamp and the ordered list of transaction ids it contains.
//! Its own hash is the sha256 over exactly those fields, so replaying the
//! same history always gives the same hashes.
//!
//! The JSON form is the one the public interface describes:
//!
//! ```json
//! {"height":0,"hash":"<hex>","parent_hash":"<hex>","state_root":"<hex>",
//!  "timestamp_ms":1790000000000,"tx_count":0,"transactions":[]}
//! ```

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::crypto::Signature;
use crate::genesis::Genesis;
use crate::hash::Hash;

/// Domain tag of the block hash.
const BLOCK_TAG: &[u8] = b"botchain:block:v1";

/// A sealed block.
///
/// Build blocks with [`Block::new`] or [`Block::genesis`]: they fill
/// `tx_count` and `hash` from the other fields. A block that arrives from
/// the outside (JSON, storage) can be checked with [`Block::is_consistent`].
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Block {
    /// Position in the chain; the genesis block is height 0.
    pub height: u64,
    /// The hash over every other field of this block.
    pub hash: Hash,
    /// The hash of the block before it; 64 zeros for genesis.
    pub parent_hash: Hash,
    /// The state root after all of this block's transfers applied.
    pub state_root: Hash,
    /// When the block was sealed, in milliseconds since the unix epoch.
    pub timestamp_ms: u64,
    /// How many transactions the block contains.
    pub tx_count: u64,
    /// The transaction ids, in the order they were applied.
    pub transactions: Vec<Signature>,
}

impl Block {
    /// Seals a block: fills in `tx_count` and the block hash.
    pub fn new(
        height: u64,
        parent_hash: Hash,
        state_root: Hash,
        timestamp_ms: u64,
        transactions: Vec<Signature>,
    ) -> Block {
        let hash = block_hash(
            height,
            &parent_hash,
            &state_root,
            timestamp_ms,
            &transactions,
        );
        Block {
            height,
            hash,
            parent_hash,
            state_root,
            timestamp_ms,
            tx_count: transactions.len() as u64,
            transactions,
        }
    }

    /// The block at height 0: no transactions, the zero parent hash and the
    /// genesis timestamp, committing to the state root of the allocations.
    pub fn genesis(genesis: &Genesis, state_root: Hash) -> Block {
        Block::new(0, Hash::ZERO, state_root, genesis.timestamp_ms, Vec::new())
    }

    /// The hash these fields produce, ignoring the stored `hash`.
    pub fn compute_hash(&self) -> Hash {
        block_hash(
            self.height,
            &self.parent_hash,
            &self.state_root,
            self.timestamp_ms,
            &self.transactions,
        )
    }

    /// True when `hash` and `tx_count` match the rest of the block.
    pub fn is_consistent(&self) -> bool {
        self.tx_count == self.transactions.len() as u64 && self.hash == self.compute_hash()
    }

    /// True when this block can follow `parent`: next height, parent hash
    /// linked and time that does not run backwards.
    pub fn follows(&self, parent: &Block) -> bool {
        self.height == parent.height.saturating_add(1)
            && self.parent_hash == parent.hash
            && self.timestamp_ms >= parent.timestamp_ms
    }

    /// True when this is a genesis block: height 0 with the zero parent.
    pub fn is_genesis(&self) -> bool {
        self.height == 0 && self.parent_hash == Hash::ZERO
    }
}

/// The tip of the chain: what a checkpoint written elsewhere commits to.
///
/// The JSON form is `{"height":<int>,"hash":"<hex>","state_root":"<hex>"}`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Checkpoint {
    /// Height of the latest sealed block.
    pub height: u64,
    /// Hash of that block.
    pub hash: Hash,
    /// State root after that block.
    pub state_root: Hash,
}

impl From<&Block> for Checkpoint {
    fn from(block: &Block) -> Self {
        Checkpoint {
            height: block.height,
            hash: block.hash,
            state_root: block.state_root,
        }
    }
}

/// The block hash: sha256 over a tag, the header fields in big endian and
/// every transaction id in order.
pub fn block_hash(
    height: u64,
    parent_hash: &Hash,
    state_root: &Hash,
    timestamp_ms: u64,
    transactions: &[Signature],
) -> Hash {
    let mut hasher = Sha256::new();
    hasher.update(BLOCK_TAG);
    hasher.update(height.to_be_bytes());
    hasher.update(parent_hash.as_bytes());
    hasher.update(state_root.as_bytes());
    hasher.update(timestamp_ms.to_be_bytes());
    hasher.update((transactions.len() as u64).to_be_bytes());
    for signature in transactions {
        hasher.update(signature.as_bytes());
    }
    let digest = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    Hash::from_bytes(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::amount::Amount;
    use crate::crypto::Keypair;
    use crate::hash::sha256;
    use crate::tx::Transfer;

    const CHAIN: &str = "botchain-devnet";

    fn signature(seed: u8) -> Signature {
        let kp = Keypair::from_secret_bytes([seed; 32]);
        let tx = Transfer::sign(CHAIN, &kp, kp.address(), Amount::from_u64(1), 0);
        tx.signature
    }

    fn genesis_doc() -> Genesis {
        Genesis {
            chain_id: CHAIN.to_string(),
            timestamp_ms: 1_790_000_000_000,
            allocations: Vec::new(),
        }
    }

    #[test]
    fn genesis_block_has_height_zero_and_a_zero_parent() {
        let block = Block::genesis(&genesis_doc(), sha256(b"root"));
        assert_eq!(block.height, 0);
        assert_eq!(block.parent_hash, Hash::ZERO);
        assert_eq!(block.parent_hash.to_hex(), "0".repeat(64));
        assert_eq!(block.timestamp_ms, 1_790_000_000_000);
        assert_eq!(block.tx_count, 0);
        assert!(block.transactions.is_empty());
        assert!(block.is_genesis());
        assert!(block.is_consistent());
    }

    #[test]
    fn hash_depends_on_every_field() {
        let base = Block::new(1, sha256(b"p"), sha256(b"s"), 10, vec![signature(1)]);
        let others = [
            Block::new(2, sha256(b"p"), sha256(b"s"), 10, vec![signature(1)]),
            Block::new(1, sha256(b"q"), sha256(b"s"), 10, vec![signature(1)]),
            Block::new(1, sha256(b"p"), sha256(b"t"), 10, vec![signature(1)]),
            Block::new(1, sha256(b"p"), sha256(b"s"), 11, vec![signature(1)]),
            Block::new(1, sha256(b"p"), sha256(b"s"), 10, vec![signature(2)]),
            Block::new(
                1,
                sha256(b"p"),
                sha256(b"s"),
                10,
                vec![signature(1), signature(2)],
            ),
            Block::new(1, sha256(b"p"), sha256(b"s"), 10, Vec::new()),
        ];
        for other in &others {
            assert_ne!(base.hash, other.hash);
        }
        // The same inputs always give the same hash.
        let same = Block::new(1, sha256(b"p"), sha256(b"s"), 10, vec![signature(1)]);
        assert_eq!(base, same);
        assert_eq!(base.hash, same.hash);
    }

    #[test]
    fn transaction_order_matters() {
        let a = Block::new(
            1,
            Hash::ZERO,
            Hash::ZERO,
            0,
            vec![signature(1), signature(2)],
        );
        let b = Block::new(
            1,
            Hash::ZERO,
            Hash::ZERO,
            0,
            vec![signature(2), signature(1)],
        );
        assert_ne!(a.hash, b.hash);
    }

    #[test]
    fn tampering_is_visible() {
        let mut block = Block::new(1, sha256(b"p"), sha256(b"s"), 10, vec![signature(1)]);
        assert!(block.is_consistent());
        block.timestamp_ms += 1;
        assert!(!block.is_consistent());

        let mut block = Block::new(1, sha256(b"p"), sha256(b"s"), 10, vec![signature(1)]);
        block.tx_count = 5;
        assert!(!block.is_consistent());
    }

    #[test]
    fn follows_checks_height_parent_and_time() {
        let genesis = Block::genesis(&genesis_doc(), sha256(b"root"));
        let next = Block::new(
            1,
            genesis.hash,
            sha256(b"s"),
            genesis.timestamp_ms + 1000,
            vec![],
        );
        assert!(next.follows(&genesis));

        let wrong_parent = Block::new(1, sha256(b"nope"), sha256(b"s"), 0, vec![]);
        assert!(!wrong_parent.follows(&genesis));
        let wrong_height = Block::new(2, genesis.hash, sha256(b"s"), genesis.timestamp_ms, vec![]);
        assert!(!wrong_height.follows(&genesis));
        let back_in_time = Block::new(
            1,
            genesis.hash,
            sha256(b"s"),
            genesis.timestamp_ms - 1,
            vec![],
        );
        assert!(!back_in_time.follows(&genesis));
    }

    #[test]
    fn json_shape_is_the_public_interface() {
        let sig = signature(3);
        let block = Block::new(7, sha256(b"p"), sha256(b"s"), 1234, vec![sig]);
        let json = serde_json::to_string(&block).expect("serialize");
        assert_eq!(
            json,
            format!(
                r#"{{"height":7,"hash":"{}","parent_hash":"{}","state_root":"{}","timestamp_ms":1234,"tx_count":1,"transactions":["{}"]}}"#,
                block.hash,
                sha256(b"p"),
                sha256(b"s"),
                sig.to_base58()
            )
        );
        let back: Block = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, block);
        assert!(back.is_consistent());
    }
}

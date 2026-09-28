//! The chain: the sealed blocks, the state they produce and the index from
//! transaction id to the block that confirmed it.
//!
//! [`Chain::seal`] is the one way the chain grows. It takes transfers in
//! order, applies the valid ones to the state, drops the invalid ones with a
//! reason, and seals a block even when nothing was applied.

use std::collections::BTreeMap;

use crate::block::{Block, Checkpoint};
use crate::crypto::Signature;
use crate::genesis::{Genesis, GenesisError};
use crate::hash::Hash;
use crate::state::{State, StateError};
use crate::tx::Transfer;

/// A transfer that made it into a block.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ConfirmedTransfer {
    /// The transfer itself.
    pub transfer: Transfer,
    /// The height of the block that contains it.
    pub block_height: u64,
}

/// Why the sequencer dropped a transfer instead of including it.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RejectReason {
    /// The same signature is already confirmed in an earlier block.
    #[error("transaction is already confirmed in block {0}")]
    AlreadyConfirmed(u64),
    /// The same signature appears twice in the same block.
    #[error("transaction appears twice in the same block")]
    DuplicateInBlock,
    /// The state machine refused it: signature, nonce or balance.
    #[error(transparent)]
    State(#[from] StateError),
}

/// A transfer the sequencer dropped, with the reason.
#[derive(Debug, PartialEq, Eq)]
pub struct Rejected {
    /// The dropped transfer.
    pub transfer: Transfer,
    /// Why it was dropped.
    pub reason: RejectReason,
}

/// The result of one sealing round.
#[derive(Debug)]
pub struct SealedBlock {
    /// The block that was sealed, empty or not.
    pub block: Block,
    /// The transfers that were applied, in block order.
    pub included: Vec<Transfer>,
    /// The transfers that were dropped, with their reasons.
    pub rejected: Vec<Rejected>,
}

/// The whole chain in memory: blocks, state and the transaction index.
#[derive(Debug)]
pub struct Chain {
    chain_id: String,
    state: State,
    blocks: Vec<Block>,
    index: BTreeMap<Signature, ConfirmedTransfer>,
}

impl Chain {
    /// Starts a chain from a genesis document: state and block 0.
    pub fn new(genesis: &Genesis) -> Result<Chain, GenesisError> {
        let state = State::from_genesis(genesis)?;
        let block = Block::genesis(genesis, state.state_root());
        Ok(Chain {
            chain_id: genesis.chain_id.clone(),
            state,
            blocks: vec![block],
            index: BTreeMap::new(),
        })
    }

    /// The chain id every signature is bound to.
    pub fn chain_id(&self) -> &str {
        &self.chain_id
    }

    /// The current account state.
    pub fn state(&self) -> &State {
        &self.state
    }

    /// The height of the latest block; genesis is 0.
    pub fn height(&self) -> u64 {
        self.head().height
    }

    /// The latest block. A chain always has at least the genesis block.
    pub fn head(&self) -> &Block {
        // `new` pushes the genesis block and nothing ever removes a block.
        self.blocks.last().unwrap_or(&GENESIS_PLACEHOLDER)
    }

    /// The block at `height`, if it exists.
    pub fn block(&self, height: u64) -> Option<&Block> {
        self.blocks.get(usize::try_from(height).ok()?)
    }

    /// The block with this hash, if it exists.
    pub fn block_by_hash(&self, hash: &Hash) -> Option<&Block> {
        self.blocks.iter().find(|block| &block.hash == hash)
    }

    /// The newest blocks first, at most `limit` of them.
    pub fn latest_blocks(&self, limit: usize) -> Vec<&Block> {
        self.blocks.iter().rev().take(limit).collect()
    }

    /// All blocks from genesis upwards.
    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    /// The confirmed transfer with this id, if the chain has it.
    pub fn transaction(&self, signature: &Signature) -> Option<&ConfirmedTransfer> {
        self.index.get(signature)
    }

    /// The tip, in the shape a checkpoint is published in.
    pub fn checkpoint(&self) -> Checkpoint {
        Checkpoint::from(self.head())
    }

    /// Seals the next block from `transfers`, in the order they are given.
    ///
    /// Valid transfers are applied to the state and land in the block; the
    /// others are dropped with a reason and never touch the state. A block is
    /// always produced, even when every transfer was dropped or there were
    /// none at all. The timestamp never runs behind the parent's.
    pub fn seal(
        &mut self,
        timestamp_ms: u64,
        transfers: impl IntoIterator<Item = Transfer>,
    ) -> SealedBlock {
        let parent = self.head().clone();
        let height = parent.height.saturating_add(1);
        let timestamp_ms = timestamp_ms.max(parent.timestamp_ms);

        let mut included: Vec<Transfer> = Vec::new();
        let mut rejected: Vec<Rejected> = Vec::new();
        let mut ids: Vec<Signature> = Vec::new();

        for transfer in transfers {
            if let Some(confirmed) = self.index.get(&transfer.signature) {
                rejected.push(Rejected {
                    transfer,
                    reason: RejectReason::AlreadyConfirmed(confirmed.block_height),
                });
                continue;
            }
            if ids.contains(&transfer.signature) {
                rejected.push(Rejected {
                    transfer,
                    reason: RejectReason::DuplicateInBlock,
                });
                continue;
            }
            match self.state.apply_transfer(&self.chain_id, &transfer) {
                Ok(()) => {
                    ids.push(transfer.signature);
                    included.push(transfer);
                }
                Err(error) => rejected.push(Rejected {
                    transfer,
                    reason: RejectReason::State(error),
                }),
            }
        }

        let block = Block::new(
            height,
            parent.hash,
            self.state.state_root(),
            timestamp_ms,
            ids,
        );
        for transfer in &included {
            self.index.insert(
                transfer.signature,
                ConfirmedTransfer {
                    transfer: *transfer,
                    block_height: height,
                },
            );
        }
        self.blocks.push(block.clone());

        SealedBlock {
            block,
            included,
            rejected,
        }
    }
}

/// Only used to keep [`Chain::head`] panic free; a real chain never hits it.
static GENESIS_PLACEHOLDER: Block = Block {
    height: 0,
    hash: Hash::ZERO,
    parent_hash: Hash::ZERO,
    state_root: Hash::ZERO,
    timestamp_ms: 0,
    tx_count: 0,
    transactions: Vec::new(),
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::amount::Amount;
    use crate::crypto::{Address, Keypair};
    use crate::genesis::Allocation;
    use crate::tx::TxError;

    const CHAIN: &str = "botchain-devnet";

    fn keypair(seed: u8) -> Keypair {
        Keypair::from_secret_bytes([seed; 32])
    }

    fn genesis(allocations: Vec<(Address, u64)>) -> Genesis {
        Genesis {
            chain_id: CHAIN.to_string(),
            timestamp_ms: 1_790_000_000_000,
            allocations: allocations
                .into_iter()
                .map(|(address, amount)| Allocation {
                    address,
                    amount: Amount::from_u64(amount),
                })
                .collect(),
        }
    }

    fn funded_chain(alice: &Keypair, amount: u64) -> Chain {
        Chain::new(&genesis(vec![(alice.address(), amount)])).expect("genesis")
    }

    #[test]
    fn a_new_chain_is_just_the_genesis_block() {
        let alice = keypair(1);
        let chain = funded_chain(&alice, 1000);
        assert_eq!(chain.height(), 0);
        assert_eq!(chain.blocks().len(), 1);
        let block = chain.head();
        assert!(block.is_genesis());
        assert_eq!(block.parent_hash, Hash::ZERO);
        assert_eq!(block.state_root, chain.state().state_root());
        assert_eq!(block.timestamp_ms, 1_790_000_000_000);
        assert_eq!(
            chain.state().balance(&alice.address()),
            Amount::from_u64(1000)
        );
        let checkpoint = chain.checkpoint();
        assert_eq!(checkpoint.height, 0);
        assert_eq!(checkpoint.hash, block.hash);
        assert_eq!(checkpoint.state_root, block.state_root);
    }

    #[test]
    fn empty_ticks_still_seal_blocks_that_link_up() {
        let alice = keypair(1);
        let mut chain = funded_chain(&alice, 10);
        let genesis_root = chain.state().state_root();

        for i in 1..=5u64 {
            let sealed = chain.seal(1_790_000_000_000 + i * 1000, Vec::new());
            assert_eq!(sealed.block.height, i);
            assert_eq!(sealed.block.tx_count, 0);
            assert!(sealed.included.is_empty());
            assert!(sealed.rejected.is_empty());
            // No transfers, so the state root does not move.
            assert_eq!(sealed.block.state_root, genesis_root);
        }
        assert_eq!(chain.height(), 5);

        // Every block links to its parent by hash and height.
        for height in 1..=5u64 {
            let block = chain.block(height).expect("block");
            let parent = chain.block(height - 1).expect("parent");
            assert!(block.follows(parent));
            assert!(block.is_consistent());
        }
        assert!(chain.block(6).is_none());
        let head_hash = chain.head().hash;
        assert_eq!(chain.block_by_hash(&head_hash).map(|b| b.height), Some(5));
    }

    #[test]
    fn valid_transfers_are_applied_and_indexed() {
        let alice = keypair(1);
        let bob = keypair(2);
        let mut chain = funded_chain(&alice, 1000);

        let t1 = Transfer::sign(CHAIN, &alice, bob.address(), Amount::from_u64(100), 0);
        let t2 = Transfer::sign(CHAIN, &alice, bob.address(), Amount::from_u64(50), 1);
        let sealed = chain.seal(1_790_000_001_000, vec![t1, t2]);

        assert_eq!(sealed.block.height, 1);
        assert_eq!(sealed.block.tx_count, 2);
        assert_eq!(sealed.block.transactions, vec![t1.signature, t2.signature]);
        assert_eq!(sealed.included, vec![t1, t2]);
        assert!(sealed.rejected.is_empty());
        assert_eq!(sealed.block.state_root, chain.state().state_root());
        assert_eq!(chain.state().balance(&bob.address()), Amount::from_u64(150));

        let confirmed = chain.transaction(&t1.signature).expect("indexed");
        assert_eq!(confirmed.block_height, 1);
        assert_eq!(confirmed.transfer, t1);
        let unknown = Transfer::sign(CHAIN, &bob, alice.address(), Amount::from_u64(1), 0);
        assert!(chain.transaction(&unknown.signature).is_none());
    }

    #[test]
    fn invalid_transfers_are_dropped_and_the_block_still_seals() {
        let alice = keypair(1);
        let bob = keypair(2);
        let stranger = keypair(3);
        let mut chain = funded_chain(&alice, 100);

        let good = Transfer::sign(CHAIN, &alice, bob.address(), Amount::from_u64(10), 0);
        let broke = Transfer::sign(CHAIN, &stranger, bob.address(), Amount::from_u64(1), 0);
        let bad_nonce = Transfer::sign(CHAIN, &alice, bob.address(), Amount::from_u64(1), 9);
        let mut tampered = Transfer::sign(CHAIN, &alice, bob.address(), Amount::from_u64(5), 1);
        tampered.amount = Amount::from_u64(6);

        let sealed = chain.seal(1_790_000_001_000, vec![good, broke, bad_nonce, tampered]);
        assert_eq!(sealed.included, vec![good]);
        assert_eq!(sealed.block.transactions, vec![good.signature]);
        assert_eq!(sealed.rejected.len(), 3);
        assert!(matches!(
            sealed.rejected[0].reason,
            RejectReason::State(StateError::InsufficientBalance { .. })
        ));
        assert!(matches!(
            sealed.rejected[1].reason,
            RejectReason::State(StateError::WrongNonce { .. })
        ));
        assert_eq!(
            sealed.rejected[2].reason,
            RejectReason::State(StateError::Tx(TxError::BadSignature))
        );
        // Dropped transfers touched neither the state nor the index.
        assert_eq!(chain.state().balance(&bob.address()), Amount::from_u64(10));
        assert!(chain.transaction(&broke.signature).is_none());
        assert_eq!(chain.height(), 1);
    }

    #[test]
    fn a_confirmed_transfer_cannot_be_replayed() {
        let alice = keypair(1);
        let bob = keypair(2);
        let mut chain = funded_chain(&alice, 100);
        let tx = Transfer::sign(CHAIN, &alice, bob.address(), Amount::from_u64(10), 0);

        let first = chain.seal(1, vec![tx]);
        assert_eq!(first.included, vec![tx]);

        let second = chain.seal(2, vec![tx]);
        assert!(second.included.is_empty());
        assert_eq!(second.rejected.len(), 1);
        assert_eq!(second.rejected[0].reason, RejectReason::AlreadyConfirmed(1));

        // The same signature twice inside one block is caught as well.
        let other = Transfer::sign(CHAIN, &alice, bob.address(), Amount::from_u64(5), 1);
        let third = chain.seal(3, vec![other, other]);
        assert_eq!(third.included, vec![other]);
        assert_eq!(third.rejected[0].reason, RejectReason::DuplicateInBlock);
        assert_eq!(chain.state().balance(&bob.address()), Amount::from_u64(15));
    }

    #[test]
    fn timestamps_never_run_backwards() {
        let mut chain = funded_chain(&keypair(1), 1);
        let genesis_time = chain.head().timestamp_ms;
        let sealed = chain.seal(0, Vec::new());
        assert_eq!(sealed.block.timestamp_ms, genesis_time);
        assert!(sealed.block.follows(chain.block(0).expect("genesis")));
    }

    /// Replaying the same transfers from genesis must give the same block
    /// hashes and the same state roots, block for block.
    #[test]
    fn replaying_the_same_history_gives_the_same_hashes() {
        let alice = keypair(1);
        let bob = keypair(2);
        let carol = keypair(3);

        let build = || {
            let mut chain =
                Chain::new(&genesis(vec![(alice.address(), 1000), (bob.address(), 10)]))
                    .expect("genesis");
            let t1 = Transfer::sign(CHAIN, &alice, bob.address(), Amount::from_u64(100), 0);
            let t2 = Transfer::sign(CHAIN, &bob, carol.address(), Amount::from_u64(40), 0);
            let bad = Transfer::sign(CHAIN, &carol, alice.address(), Amount::from_u64(999), 0);
            let t3 = Transfer::sign(CHAIN, &alice, carol.address(), Amount::from_u64(10), 1);
            chain.seal(1_790_000_001_000, vec![t1, t2]);
            chain.seal(1_790_000_002_000, vec![bad]);
            chain.seal(1_790_000_003_000, Vec::new());
            chain.seal(1_790_000_004_000, vec![t3]);
            chain
        };

        let one = build();
        let two = build();
        assert_eq!(one.height(), 4);
        assert_eq!(one.blocks(), two.blocks());
        assert_eq!(one.state().state_root(), two.state().state_root());
        assert_eq!(one.head().state_root, one.state().state_root());
        assert_eq!(one.checkpoint(), two.checkpoint());

        // The chain is a real chain: every block links to the one before it.
        for (parent, block) in one.blocks().iter().zip(one.blocks().iter().skip(1)) {
            assert!(block.follows(parent));
            assert!(block.is_consistent());
        }
        assert_eq!(one.state().balance(&carol.address()), Amount::from_u64(50));
    }

    #[test]
    fn latest_blocks_are_newest_first() {
        let mut chain = funded_chain(&keypair(1), 1);
        for i in 1..=3 {
            chain.seal(i, Vec::new());
        }
        let latest = chain.latest_blocks(2);
        assert_eq!(latest.len(), 2);
        assert_eq!(latest[0].height, 3);
        assert_eq!(latest[1].height, 2);
        assert_eq!(chain.latest_blocks(100).len(), 4);
    }
}

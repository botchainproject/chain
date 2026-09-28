//! The sequencer: a pending queue in front of the chain, and a tick that
//! seals a block from it.
//!
//! Transfers are accepted into the queue after the checks that need no
//! ordering (signature, duplicates, an obviously stale nonce). Every tick the
//! sequencer takes the queue in order, hands it to [`Chain::seal`], and a
//! block comes out — empty when nothing was pending or everything was
//! dropped.

use std::collections::{BTreeSet, VecDeque};

use crate::chain::{Chain, SealedBlock};
use crate::crypto::Signature;
use crate::genesis::{Genesis, GenesisError};
use crate::state::StateError;
use crate::tx::{Transfer, TxError};

/// How many transfers the queue holds before it pushes back.
pub const DEFAULT_MEMPOOL_LIMIT: usize = 10_000;
/// How many transfers one block takes at most.
pub const DEFAULT_BLOCK_LIMIT: usize = 1_000;

/// Why a transfer was not accepted into the pending queue.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SubmitError {
    /// The transfer is not valid on its own (amount, sender, signature).
    #[error(transparent)]
    Tx(#[from] TxError),
    /// The same signature is already pending.
    #[error("transaction is already pending")]
    AlreadyPending,
    /// The same signature is already in a block.
    #[error("transaction is already confirmed in block {0}")]
    AlreadyConfirmed(u64),
    /// The nonce is below the sender's current nonce: it can never apply.
    #[error("nonce {got} is already used by {address}, which is at {expected}")]
    StaleNonce {
        /// The sender.
        address: String,
        /// The sender's current nonce.
        expected: u64,
        /// The nonce the transfer carries.
        got: u64,
    },
    /// The queue is full.
    #[error("mempool is full ({0} transactions)")]
    MempoolFull(usize),
}

/// A chain plus the transfers waiting to go into the next block.
#[derive(Debug)]
pub struct Sequencer {
    chain: Chain,
    pending: VecDeque<Transfer>,
    pending_ids: BTreeSet<Signature>,
    mempool_limit: usize,
    block_limit: usize,
}

impl Sequencer {
    /// Starts a sequencer on a fresh chain built from `genesis`.
    pub fn new(genesis: &Genesis) -> Result<Sequencer, GenesisError> {
        Ok(Sequencer::from_chain(Chain::new(genesis)?))
    }

    /// Wraps an existing chain, with an empty queue.
    pub fn from_chain(chain: Chain) -> Sequencer {
        Sequencer {
            chain,
            pending: VecDeque::new(),
            pending_ids: BTreeSet::new(),
            mempool_limit: DEFAULT_MEMPOOL_LIMIT,
            block_limit: DEFAULT_BLOCK_LIMIT,
        }
    }

    /// Sets how many transfers may wait and how many go into one block.
    pub fn with_limits(mut self, mempool_limit: usize, block_limit: usize) -> Sequencer {
        self.mempool_limit = mempool_limit;
        self.block_limit = block_limit.max(1);
        self
    }

    /// The chain behind the queue.
    pub fn chain(&self) -> &Chain {
        &self.chain
    }

    /// The chain behind the queue, mutably.
    pub fn chain_mut(&mut self) -> &mut Chain {
        &mut self.chain
    }

    /// How many transfers are waiting.
    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    /// True when nothing is waiting.
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// The waiting transfers, in the order they will be applied.
    pub fn pending(&self) -> impl Iterator<Item = &Transfer> {
        self.pending.iter()
    }

    /// The pending transfer with this id, if it is still waiting.
    pub fn pending_transfer(&self, signature: &Signature) -> Option<&Transfer> {
        self.pending
            .iter()
            .find(|transfer| &transfer.signature == signature)
    }

    /// Accepts a transfer into the queue and returns its id.
    ///
    /// Only checks that need no ordering happen here; whether the transfer
    /// really applies is decided when the block is sealed.
    pub fn submit(&mut self, transfer: Transfer) -> Result<Signature, SubmitError> {
        transfer.verify(self.chain.chain_id())?;
        if self.pending_ids.contains(&transfer.signature) {
            return Err(SubmitError::AlreadyPending);
        }
        if let Some(confirmed) = self.chain.transaction(&transfer.signature) {
            return Err(SubmitError::AlreadyConfirmed(confirmed.block_height));
        }
        let expected = self.chain.state().nonce(&transfer.from);
        if transfer.nonce < expected {
            return Err(SubmitError::StaleNonce {
                address: transfer.from.to_base58(),
                expected,
                got: transfer.nonce,
            });
        }
        if self.pending.len() >= self.mempool_limit {
            return Err(SubmitError::MempoolFull(self.mempool_limit));
        }
        let signature = transfer.signature;
        self.pending_ids.insert(signature);
        self.pending.push_back(transfer);
        Ok(signature)
    }

    /// Seals the next block from the queue, empty or not.
    ///
    /// Takes at most `block_limit` pending transfers in order. Dropped
    /// transfers do not go back into the queue: they could never apply in
    /// this order, and a client can always send a fresh one.
    pub fn tick(&mut self, timestamp_ms: u64) -> SealedBlock {
        let take = self.block_limit.min(self.pending.len());
        let batch: Vec<Transfer> = self.pending.drain(..take).collect();
        for transfer in &batch {
            self.pending_ids.remove(&transfer.signature);
        }
        self.chain.seal(timestamp_ms, batch)
    }
}

/// Convenience: why a transfer already in the queue would fail right now.
pub fn dry_run(sequencer: &Sequencer, transfer: &Transfer) -> Result<(), StateError> {
    sequencer
        .chain()
        .state()
        .validate_transfer(sequencer.chain().chain_id(), transfer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::amount::Amount;
    use crate::crypto::{Address, Keypair};
    use crate::genesis::Allocation;

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

    fn sequencer(alice: &Keypair, amount: u64) -> Sequencer {
        Sequencer::new(&genesis(vec![(alice.address(), amount)])).expect("genesis")
    }

    #[test]
    fn ticks_seal_empty_blocks() {
        let mut seq = sequencer(&keypair(1), 10);
        assert_eq!(seq.chain().height(), 0);
        for i in 1..=3u64 {
            let sealed = seq.tick(1_790_000_000_000 + i * 1000);
            assert_eq!(sealed.block.height, i);
            assert_eq!(sealed.block.tx_count, 0);
        }
        assert_eq!(seq.chain().height(), 3);
        assert!(seq.is_empty());
    }

    #[test]
    fn submitted_transfers_land_in_the_next_block() {
        let alice = keypair(1);
        let bob = keypair(2);
        let mut seq = sequencer(&alice, 1000);

        let t1 = Transfer::sign(CHAIN, &alice, bob.address(), Amount::from_u64(100), 0);
        let t2 = Transfer::sign(CHAIN, &alice, bob.address(), Amount::from_u64(200), 1);
        assert_eq!(seq.submit(t1), Ok(t1.signature));
        assert_eq!(seq.submit(t2), Ok(t2.signature));
        assert_eq!(seq.pending_len(), 2);
        assert_eq!(seq.pending_transfer(&t1.signature), Some(&t1));

        let sealed = seq.tick(1_790_000_001_000);
        assert_eq!(sealed.block.transactions, vec![t1.signature, t2.signature]);
        assert!(seq.is_empty());
        assert_eq!(
            seq.chain().state().balance(&bob.address()),
            Amount::from_u64(300)
        );
        assert_eq!(
            seq.chain()
                .transaction(&t2.signature)
                .map(|c| c.block_height),
            Some(1)
        );
        assert!(seq.pending_transfer(&t1.signature).is_none());
    }

    #[test]
    fn bad_transfers_are_refused_at_submit() {
        let alice = keypair(1);
        let bob = keypair(2);
        let mut seq = sequencer(&alice, 1000);

        let mut tampered = Transfer::sign(CHAIN, &alice, bob.address(), Amount::from_u64(1), 0);
        tampered.nonce = 1;
        assert_eq!(
            seq.submit(tampered),
            Err(SubmitError::Tx(TxError::BadSignature))
        );

        let zero = Transfer::sign(CHAIN, &alice, bob.address(), Amount::ZERO, 0);
        assert_eq!(seq.submit(zero), Err(SubmitError::Tx(TxError::ZeroAmount)));

        let wrong_chain = Transfer::sign(
            "botchain-mainnet",
            &alice,
            bob.address(),
            Amount::from_u64(1),
            0,
        );
        assert_eq!(
            seq.submit(wrong_chain),
            Err(SubmitError::Tx(TxError::BadSignature))
        );
        assert!(seq.is_empty());
    }

    #[test]
    fn duplicates_and_stale_nonces_are_refused() {
        let alice = keypair(1);
        let bob = keypair(2);
        let mut seq = sequencer(&alice, 1000);

        let tx = Transfer::sign(CHAIN, &alice, bob.address(), Amount::from_u64(10), 0);
        assert_eq!(seq.submit(tx), Ok(tx.signature));
        assert_eq!(seq.submit(tx), Err(SubmitError::AlreadyPending));

        seq.tick(1);
        assert_eq!(seq.submit(tx), Err(SubmitError::AlreadyConfirmed(1)));

        let stale = Transfer::sign(CHAIN, &alice, bob.address(), Amount::from_u64(11), 0);
        assert_eq!(
            seq.submit(stale),
            Err(SubmitError::StaleNonce {
                address: alice.address().to_base58(),
                expected: 1,
                got: 0,
            })
        );
        // A future nonce is fine: it may become valid.
        let future = Transfer::sign(CHAIN, &alice, bob.address(), Amount::from_u64(1), 5);
        assert!(seq.submit(future).is_ok());
    }

    #[test]
    fn a_full_mempool_pushes_back() {
        let alice = keypair(1);
        let bob = keypair(2);
        let mut seq = sequencer(&alice, 1000).with_limits(2, 10);
        for nonce in 0..2 {
            let tx = Transfer::sign(CHAIN, &alice, bob.address(), Amount::from_u64(1), nonce);
            assert!(seq.submit(tx).is_ok());
        }
        let extra = Transfer::sign(CHAIN, &alice, bob.address(), Amount::from_u64(1), 2);
        assert_eq!(seq.submit(extra), Err(SubmitError::MempoolFull(2)));
        seq.tick(1);
        assert!(seq.submit(extra).is_ok());
    }

    #[test]
    fn a_block_takes_at_most_the_block_limit() {
        let alice = keypair(1);
        let bob = keypair(2);
        let mut seq = sequencer(&alice, 1000).with_limits(100, 2);
        for nonce in 0..5 {
            let tx = Transfer::sign(CHAIN, &alice, bob.address(), Amount::from_u64(1), nonce);
            seq.submit(tx).expect("submit");
        }
        let first = seq.tick(1);
        assert_eq!(first.block.tx_count, 2);
        assert_eq!(seq.pending_len(), 3);
        let second = seq.tick(2);
        assert_eq!(second.block.tx_count, 2);
        let third = seq.tick(3);
        assert_eq!(third.block.tx_count, 1);
        assert!(seq.is_empty());
        assert_eq!(
            seq.chain().state().balance(&bob.address()),
            Amount::from_u64(5)
        );
    }

    #[test]
    fn transfers_that_cannot_apply_are_dropped_at_seal() {
        let alice = keypair(1);
        let bob = keypair(2);
        let mut seq = sequencer(&alice, 10);

        // Accepted into the queue (the nonce is not stale) but unaffordable.
        let too_much = Transfer::sign(CHAIN, &alice, bob.address(), Amount::from_u64(50), 0);
        seq.submit(too_much).expect("submit");
        let sealed = seq.tick(1);
        assert!(sealed.included.is_empty());
        assert_eq!(sealed.rejected.len(), 1);
        assert_eq!(sealed.block.tx_count, 0);
        assert_eq!(seq.chain().height(), 1);
        // Dropped, not requeued.
        assert!(seq.is_empty());
        assert!(dry_run(&seq, &too_much).is_err());
    }

    #[test]
    fn the_same_stream_of_ticks_replays_identically() {
        let alice = keypair(1);
        let bob = keypair(2);
        let carol = keypair(3);

        let run = || {
            let mut seq = Sequencer::new(&genesis(vec![(alice.address(), 500)])).expect("genesis");
            let t1 = Transfer::sign(CHAIN, &alice, bob.address(), Amount::from_u64(100), 0);
            let t2 = Transfer::sign(CHAIN, &bob, carol.address(), Amount::from_u64(30), 0);
            let t3 = Transfer::sign(CHAIN, &alice, carol.address(), Amount::from_u64(70), 1);
            seq.submit(t1).expect("submit");
            seq.tick(1_790_000_001_000);
            seq.submit(t2).expect("submit");
            seq.submit(t3).expect("submit");
            seq.tick(1_790_000_002_000);
            seq.tick(1_790_000_003_000);
            seq
        };

        let one = run();
        let two = run();
        assert_eq!(one.chain().blocks(), two.chain().blocks());
        assert_eq!(one.chain().checkpoint(), two.chain().checkpoint());
        assert_eq!(
            one.chain().state().state_root(),
            two.chain().state().state_root()
        );
        assert_eq!(one.chain().height(), 3);
        assert_eq!(
            one.chain().state().balance(&carol.address()),
            Amount::from_u64(100)
        );
    }
}

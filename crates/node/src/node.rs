//! The running node: a sequencer behind a lock, plus the block clock.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use botchain_core::{
    AccountView, Address, Amount, Block, Chain, Checkpoint, Genesis, Sequencer, Signature,
    SubmitError, Transfer, TxView,
};
use tokio::sync::Mutex;

/// A node, shared between the block clock and every RPC request.
#[derive(Debug)]
pub struct Node {
    chain_id: String,
    block_ms: u64,
    sequencer: Mutex<Sequencer>,
}

/// The handle the RPC server and the block clock share.
pub type SharedNode = Arc<Node>;

impl Node {
    /// Builds a node whose chain starts at `genesis`.
    pub fn from_genesis(genesis: &Genesis, block_ms: u64) -> anyhow::Result<SharedNode> {
        let chain = Chain::new(genesis)?;
        Ok(Node::from_chain(chain, block_ms))
    }

    /// Builds a node on top of an existing chain.
    pub fn from_chain(chain: Chain, block_ms: u64) -> SharedNode {
        Arc::new(Node {
            chain_id: chain.chain_id().to_string(),
            block_ms,
            sequencer: Mutex::new(Sequencer::from_chain(chain)),
        })
    }

    /// The chain id every signature is bound to.
    pub fn chain_id(&self) -> &str {
        &self.chain_id
    }

    /// Milliseconds between two sealed blocks.
    pub fn block_ms(&self) -> u64 {
        self.block_ms
    }

    /// The height of the latest block; genesis is 0.
    pub async fn height(&self) -> u64 {
        self.sequencer.lock().await.chain().height()
    }

    /// The block at `height`, if the chain has it.
    pub async fn block(&self, height: u64) -> Option<Block> {
        self.sequencer.lock().await.chain().block(height).cloned()
    }

    /// The newest blocks first, at most `limit` of them.
    pub async fn latest_blocks(&self, limit: usize) -> Vec<Block> {
        self.sequencer
            .lock()
            .await
            .chain()
            .latest_blocks(limit)
            .into_iter()
            .cloned()
            .collect()
    }

    /// The balance of an address; zero when it is unknown.
    pub async fn balance(&self, address: &Address) -> Amount {
        self.sequencer.lock().await.chain().state().balance(address)
    }

    /// The account of an address; zero balance and nonce when unknown.
    pub async fn account(&self, address: &Address) -> AccountView {
        self.sequencer
            .lock()
            .await
            .chain()
            .state()
            .account_view(address)
    }

    /// A transaction by id: confirmed, still pending, or unknown.
    pub async fn transaction(&self, signature: &Signature) -> Option<TxView> {
        let sequencer = self.sequencer.lock().await;
        if let Some(confirmed) = sequencer.chain().transaction(signature) {
            return Some(TxView::confirmed(
                &confirmed.transfer,
                confirmed.block_height,
            ));
        }
        sequencer.pending_transfer(signature).map(TxView::pending)
    }

    /// The tip of the chain, in checkpoint shape.
    pub async fn checkpoint(&self) -> Checkpoint {
        self.sequencer.lock().await.chain().checkpoint()
    }

    /// How many transfers are waiting for the next block.
    pub async fn pending_len(&self) -> usize {
        self.sequencer.lock().await.pending_len()
    }

    /// Puts a transfer into the mempool and returns its id.
    pub async fn submit(&self, transfer: Transfer) -> Result<Signature, SubmitError> {
        let result = self.sequencer.lock().await.submit(transfer);
        match &result {
            Ok(signature) => println!(
                "tx accepted {} from={} amount={} nonce={}",
                signature.to_base58(),
                transfer.from.to_base58(),
                transfer.amount,
                transfer.nonce
            ),
            Err(error) => println!("tx rejected {}: {error}", transfer.id()),
        }
        result
    }

    /// Seals one block with the current wall clock time.
    pub async fn tick(&self) -> Block {
        let mut sequencer = self.sequencer.lock().await;
        let sealed = sequencer.tick(now_ms());
        for rejected in &sealed.rejected {
            println!("tx dropped {}: {}", rejected.transfer.id(), rejected.reason);
        }
        println!(
            "block {} hash={} txs={} state_root={}",
            sealed.block.height, sealed.block.hash, sealed.block.tx_count, sealed.block.state_root
        );
        sealed.block
    }
}

/// Milliseconds since the unix epoch, 0 if the clock is before it.
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// Seals a block every `block_ms` forever.
pub async fn run_block_clock(node: SharedNode) {
    let period = std::time::Duration::from_millis(node.block_ms().max(1));
    let mut ticker = tokio::time::interval(period);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // The first tick completes immediately; skip it so block 1 is one
    // period after the start.
    ticker.tick().await;
    loop {
        ticker.tick().await;
        node.tick().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use botchain_core::{Allocation, Keypair};

    const CHAIN: &str = "botchain-devnet";

    fn genesis(address: Address, amount: u64) -> Genesis {
        Genesis {
            chain_id: CHAIN.to_string(),
            timestamp_ms: 1_790_000_000_000,
            allocations: vec![Allocation {
                address,
                amount: Amount::from_u64(amount),
            }],
        }
    }

    #[tokio::test]
    async fn a_fresh_node_is_at_genesis() {
        let alice = Keypair::from_secret_bytes([1u8; 32]);
        let node = Node::from_genesis(&genesis(alice.address(), 100), 1000).expect("node");
        assert_eq!(node.height().await, 0);
        assert_eq!(node.balance(&alice.address()).await, Amount::from_u64(100));
        let block = node.block(0).await.expect("genesis block");
        assert!(block.is_genesis());
        assert_eq!(node.block(1).await, None);
        assert_eq!(node.checkpoint().await.height, 0);
    }

    #[tokio::test]
    async fn ticks_make_blocks_and_transfers_confirm() {
        let alice = Keypair::from_secret_bytes([1u8; 32]);
        let bob = Keypair::from_secret_bytes([2u8; 32]);
        let node = Node::from_genesis(&genesis(alice.address(), 100), 1000).expect("node");

        let tx = Transfer::sign(CHAIN, &alice, bob.address(), Amount::from_u64(30), 0);
        let id = node.submit(tx).await.expect("submit");
        assert_eq!(node.pending_len().await, 1);
        let view = node.transaction(&id).await.expect("pending");
        assert_eq!(view.block_height, None);

        let block = node.tick().await;
        assert_eq!(block.height, 1);
        assert_eq!(block.tx_count, 1);
        assert_eq!(node.balance(&bob.address()).await, Amount::from_u64(30));
        let view = node.transaction(&id).await.expect("confirmed");
        assert_eq!(view.block_height, Some(1));
        assert_eq!(node.account(&alice.address()).await.nonce, 1);
        assert_eq!(node.latest_blocks(10).await.len(), 2);
    }
}

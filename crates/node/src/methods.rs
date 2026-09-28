//! The RPC methods of the fixed interface.

use botchain_core::{Address, Signature, SubmitError, Transfer};
use serde_json::{json, Value};

use crate::jsonrpc::{Params, RpcError};
use crate::node::SharedNode;

/// Default number of blocks `getLatestBlocks` returns.
pub const DEFAULT_BLOCK_LIMIT: u64 = 20;
/// Largest number of blocks `getLatestBlocks` returns.
pub const MAX_BLOCK_LIMIT: u64 = 100;

/// Runs one method call against the node.
pub async fn dispatch(node: &SharedNode, method: &str, params: Params) -> Result<Value, RpcError> {
    match method {
        "getHealth" => Ok(json!("ok")),
        "getBlockHeight" => Ok(json!(node.height().await)),
        "getBlock" => {
            let height = params.u64("height")?;
            Ok(to_value(node.block(height).await)?)
        }
        "getLatestBlocks" => {
            let limit =
                params.u64_in_range("limit", 1, MAX_BLOCK_LIMIT, DEFAULT_BLOCK_LIMIT)? as usize;
            to_value(node.latest_blocks(limit).await)
        }
        "getBalance" => {
            let address = address(&params)?;
            Ok(json!(node.balance(&address).await.to_string()))
        }
        "getAccount" => {
            let address = address(&params)?;
            to_value(node.account(&address).await)
        }
        "getTransaction" => {
            let raw = params.string("signature")?;
            let signature = Signature::parse(raw)
                .map_err(|e| RpcError::invalid_params(format!("invalid signature: {e}")))?;
            to_value(node.transaction(&signature).await)
        }
        "sendTransaction" => send_transaction(node, params).await,
        "getCheckpoint" => to_value(node.checkpoint().await),
        "getChainInfo" => Ok(json!({
            "chain_id": node.chain_id(),
            "height": node.height().await,
            "block_ms": node.block_ms(),
            "pending": node.pending_len().await,
        })),
        other => Err(RpcError::method_not_found(other)),
    }
}

/// Every method this node answers, for documentation and tests.
pub const METHODS: &[&str] = &[
    "getHealth",
    "getBlockHeight",
    "getBlock",
    "getLatestBlocks",
    "getTransaction",
    "getBalance",
    "getAccount",
    "sendTransaction",
    "getCheckpoint",
    "getChainInfo",
];

async fn send_transaction(node: &SharedNode, params: Params) -> Result<Value, RpcError> {
    let raw = params.require("tx")?;
    let transfer: Transfer = serde_json::from_value(raw.clone())
        .map_err(|e| RpcError::invalid_params(format!("invalid transfer: {e}")))?;
    match node.submit(transfer).await {
        Ok(signature) => Ok(json!(signature.to_base58())),
        Err(error) => Err(submit_error(&error)),
    }
}

fn submit_error(error: &SubmitError) -> RpcError {
    let code = match error {
        SubmitError::Tx(_) => -32001,
        SubmitError::AlreadyPending | SubmitError::AlreadyConfirmed(_) => -32002,
        SubmitError::StaleNonce { .. } => -32003,
        SubmitError::MempoolFull(_) => -32004,
    };
    RpcError::new(code, error.to_string())
}

fn address(params: &Params) -> Result<Address, RpcError> {
    let raw = params.string("address")?;
    Address::parse(raw).map_err(|e| RpcError::invalid_params(format!("invalid address: {e}")))
}

fn to_value<T: serde::Serialize>(value: T) -> Result<Value, RpcError> {
    serde_json::to_value(value)
        .map_err(|e| RpcError::server_error(format!("could not encode result: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::Node;
    use botchain_core::{Allocation, Amount, Genesis, Keypair};

    const CHAIN: &str = "botchain-devnet";

    fn node(funded: &Keypair, amount: u64) -> SharedNode {
        let genesis = Genesis {
            chain_id: CHAIN.to_string(),
            timestamp_ms: 1_790_000_000_000,
            allocations: vec![Allocation {
                address: funded.address(),
                amount: Amount::from_u64(amount),
            }],
        };
        Node::from_genesis(&genesis, 1000).expect("node")
    }

    async fn call(node: &SharedNode, method: &str, params: Value) -> Result<Value, RpcError> {
        let params = Params::from_value(Some(&params)).expect("params");
        dispatch(node, method, params).await
    }

    #[tokio::test]
    async fn health_height_and_blocks() {
        let alice = Keypair::from_secret_bytes([1u8; 32]);
        let node = node(&alice, 1000);

        assert_eq!(call(&node, "getHealth", json!({})).await, Ok(json!("ok")));
        assert_eq!(call(&node, "getBlockHeight", json!({})).await, Ok(json!(0)));

        let genesis = call(&node, "getBlock", json!({"height": 0}))
            .await
            .expect("block");
        assert_eq!(genesis["height"], json!(0));
        assert_eq!(genesis["parent_hash"], json!("0".repeat(64)));
        assert_eq!(genesis["tx_count"], json!(0));
        assert_eq!(genesis["transactions"], json!([]));
        assert_eq!(
            call(&node, "getBlock", json!({"height": 9})).await,
            Ok(Value::Null)
        );

        node.tick().await;
        let latest = call(&node, "getLatestBlocks", json!({}))
            .await
            .expect("blocks");
        let blocks = latest.as_array().expect("array");
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0]["height"], json!(1));
        assert_eq!(blocks[1]["height"], json!(0));
        let one = call(&node, "getLatestBlocks", json!({"limit": 1}))
            .await
            .expect("blocks");
        assert_eq!(one.as_array().map(Vec::len), Some(1));
    }

    #[tokio::test]
    async fn balances_and_accounts() {
        let alice = Keypair::from_secret_bytes([1u8; 32]);
        let stranger = Keypair::from_secret_bytes([9u8; 32]).address().to_base58();
        let node = node(&alice, 1000);

        assert_eq!(
            call(
                &node,
                "getBalance",
                json!({"address": alice.address().to_base58()})
            )
            .await,
            Ok(json!("1000"))
        );
        assert_eq!(
            call(&node, "getBalance", json!({"address": stranger})).await,
            Ok(json!("0"))
        );
        assert_eq!(
            call(&node, "getAccount", json!({"address": stranger})).await,
            Ok(json!({"address": stranger, "balance": "0", "nonce": 0}))
        );
    }

    #[tokio::test]
    async fn bad_params_and_unknown_methods() {
        let node = node(&Keypair::from_secret_bytes([1u8; 32]), 1);
        assert_eq!(
            call(&node, "nope", json!({}))
                .await
                .expect_err("unknown")
                .code,
            -32601
        );
        assert_eq!(
            call(&node, "getBlock", json!({}))
                .await
                .expect_err("missing")
                .code,
            -32602
        );
        assert_eq!(
            call(&node, "getBlock", json!({"height": -1}))
                .await
                .expect_err("negative")
                .code,
            -32602
        );
        assert_eq!(
            call(&node, "getBalance", json!({"address": "not base58 !!"}))
                .await
                .expect_err("bad address")
                .code,
            -32602
        );
        assert_eq!(
            call(&node, "getLatestBlocks", json!({"limit": 0}))
                .await
                .expect_err("out of range")
                .code,
            -32602
        );
    }

    #[tokio::test]
    async fn send_and_read_a_transaction() {
        let alice = Keypair::from_secret_bytes([1u8; 32]);
        let bob = Keypair::from_secret_bytes([2u8; 32]);
        let node = node(&alice, 1000);

        let tx = Transfer::sign(CHAIN, &alice, bob.address(), Amount::from_u64(250), 0);
        let raw = serde_json::to_value(tx).expect("json");
        let id = call(&node, "sendTransaction", json!({"tx": raw.clone()}))
            .await
            .expect("accepted");
        assert_eq!(id, json!(tx.signature.to_base58()));

        let pending = call(&node, "getTransaction", json!({"signature": tx.id()}))
            .await
            .expect("pending");
        assert_eq!(pending["status"], json!("pending"));
        assert_eq!(pending["block_height"], Value::Null);

        // The same transfer twice is refused.
        let again = call(&node, "sendTransaction", json!({"tx": raw}))
            .await
            .expect_err("duplicate");
        assert_eq!(again.code, -32002);

        node.tick().await;
        let confirmed = call(&node, "getTransaction", json!({"signature": tx.id()}))
            .await
            .expect("confirmed");
        assert_eq!(confirmed["status"], json!("confirmed"));
        assert_eq!(confirmed["block_height"], json!(1));
        assert_eq!(confirmed["amount"], json!("250"));

        let checkpoint = call(&node, "getCheckpoint", json!({})).await.expect("tip");
        assert_eq!(checkpoint["height"], json!(1));
        assert!(checkpoint["hash"].as_str().map(str::len) == Some(64));
    }

    #[tokio::test]
    async fn a_bad_transfer_is_rejected_with_a_server_code() {
        let alice = Keypair::from_secret_bytes([1u8; 32]);
        let bob = Keypair::from_secret_bytes([2u8; 32]);
        let node = node(&alice, 1000);

        let mut tampered = Transfer::sign(CHAIN, &alice, bob.address(), Amount::from_u64(1), 0);
        tampered.amount = Amount::from_u64(2);
        let error = call(
            &node,
            "sendTransaction",
            json!({"tx": serde_json::to_value(tampered).expect("json")}),
        )
        .await
        .expect_err("bad signature");
        assert_eq!(error.code, -32001);

        let error = call(&node, "sendTransaction", json!({"tx": {"from": "x"}}))
            .await
            .expect_err("bad shape");
        assert_eq!(error.code, -32602);
        assert_eq!(
            call(&node, "getTransaction", json!({"signature": "zzz!!"}))
                .await
                .expect_err("bad signature string")
                .code,
            -32602
        );
    }
}

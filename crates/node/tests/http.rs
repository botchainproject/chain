//! End to end: a real HTTP POST on `/` against the RPC server.

use botchain_core::{Allocation, Amount, Genesis, Keypair, Transfer};
use botchain_node::node::Node;
use botchain_node::rpc::router;
use serde_json::{json, Value};
use std::io::{Read, Write};
use tokio::net::TcpListener;

const CHAIN: &str = "botchain-devnet";

/// Starts the RPC server on an ephemeral port and returns its address.
async fn start(funded: &Keypair) -> std::net::SocketAddr {
    let genesis = Genesis {
        chain_id: CHAIN.to_string(),
        timestamp_ms: 1_790_000_000_000,
        allocations: vec![Allocation {
            address: funded.address(),
            amount: Amount::from_u64(1_000_000),
        }],
    };
    let node = Node::from_genesis(&genesis, 50).expect("node");
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("bind");
    let address = listener.local_addr().expect("addr");
    let app = router(node.clone());
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    tokio::spawn(botchain_node::node::run_block_clock(node));
    address
}

/// Sends one JSON-RPC call over a fresh connection and returns the response.
async fn call(address: std::net::SocketAddr, method: &str, params: Value) -> Value {
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).to_string();
    let request = format!(
        "POST / HTTP/1.1\r\nHost: {address}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    // A plain blocking client on a blocking thread: the runtime stays free
    // to drive the server while we wait.
    let text = tokio::task::spawn_blocking(move || {
        let mut stream = std::net::TcpStream::connect(address).expect("connect");
        stream.write_all(request.as_bytes()).expect("write request");
        let mut response = Vec::new();
        stream.read_to_end(&mut response).expect("read response");
        String::from_utf8(response).expect("utf8")
    })
    .await
    .expect("client thread");
    let (head, body) = text.split_once("\r\n\r\n").expect("headers and body");
    assert!(head.starts_with("HTTP/1.1 200 OK"), "{head}");
    serde_json::from_str(body.trim()).expect("json body")
}

#[tokio::test]
async fn the_server_answers_json_rpc_over_http() {
    let alice = Keypair::from_secret_bytes([1u8; 32]);
    let bob = Keypair::from_secret_bytes([2u8; 32]);
    let address = start(&alice).await;

    assert_eq!(call(address, "getHealth", json!({})).await["result"], "ok");

    let genesis = call(address, "getBlock", json!({"height": 0})).await;
    assert_eq!(genesis["result"]["height"], json!(0));
    assert_eq!(genesis["result"]["parent_hash"], json!("0".repeat(64)));

    let unknown = call(address, "getWeather", json!({})).await;
    assert_eq!(unknown["error"]["code"], json!(-32601));

    // The clock keeps sealing blocks while we are connected.
    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    let height = call(address, "getBlockHeight", json!({})).await["result"]
        .as_u64()
        .expect("height");
    assert!(height >= 1, "no blocks after 250ms: {height}");

    // A transfer sent over HTTP shows up in the balances.
    let tx = Transfer::sign(CHAIN, &alice, bob.address(), Amount::from_u64(777), 0);
    let sent = call(
        address,
        "sendTransaction",
        json!({"tx": serde_json::to_value(tx).expect("json")}),
    )
    .await;
    assert_eq!(sent["result"], json!(tx.id()));
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    let balance = call(
        address,
        "getBalance",
        json!({"address": bob.address().to_base58()}),
    )
    .await;
    assert_eq!(balance["result"], json!("777"));
}

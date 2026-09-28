//! The JSON-RPC 2.0 server: HTTP POST on `/`.

use std::net::{Ipv4Addr, SocketAddr};

use axum::body::Bytes;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{json, Value};

use crate::jsonrpc::{failure, success, Params, RpcError, INVALID_REQUEST, PARSE_ERROR};
use crate::methods::dispatch;
use crate::node::SharedNode;

/// The HTTP router: one JSON-RPC endpoint on `/`.
pub fn router(node: SharedNode) -> Router {
    Router::new()
        .route("/", post(handler).get(hello))
        .with_state(node)
}

/// Binds 127.0.0.1:`port` and serves until the process stops.
pub async fn serve(node: SharedNode, port: u16) -> anyhow::Result<()> {
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let listener = tokio::net::TcpListener::bind(address).await?;
    println!("rpc listening on http://{address}");
    axum::serve(listener, router(node)).await?;
    Ok(())
}

async fn hello() -> Response {
    Json(json!({"name": "botchain-node", "rpc": "jsonrpc 2.0 over POST /"})).into_response()
}

async fn handler(State(node): State<SharedNode>, body: Bytes) -> Response {
    let Ok(request) = serde_json::from_slice::<Value>(&body) else {
        let error = RpcError::new(PARSE_ERROR, "request body is not valid JSON");
        return Json(failure(Value::Null, &error)).into_response();
    };
    Json(handle_request(&node, request).await).into_response()
}

/// Answers one JSON-RPC request value, single call or batch.
pub async fn handle_request(node: &SharedNode, request: Value) -> Value {
    match request {
        Value::Array(calls) if calls.is_empty() => {
            let error = RpcError::new(INVALID_REQUEST, "batch must not be empty");
            failure(Value::Null, &error)
        }
        Value::Array(calls) => {
            let mut out = Vec::with_capacity(calls.len());
            for call in calls {
                out.push(handle_call(node, call).await);
            }
            Value::Array(out)
        }
        other => handle_call(node, other).await,
    }
}

async fn handle_call(node: &SharedNode, call: Value) -> Value {
    let id = call.get("id").cloned().unwrap_or(Value::Null);
    let Some(object) = call.as_object() else {
        let error = RpcError::new(INVALID_REQUEST, "request must be a JSON object");
        return failure(id, &error);
    };
    if let Some(version) = object.get("jsonrpc").and_then(Value::as_str) {
        if version != "2.0" {
            let error = RpcError::new(INVALID_REQUEST, "jsonrpc must be \"2.0\"");
            return failure(id, &error);
        }
    }
    let Some(method) = object.get("method").and_then(Value::as_str) else {
        let error = RpcError::new(INVALID_REQUEST, "request needs a method name");
        return failure(id, &error);
    };
    let params = match Params::from_value(object.get("params")) {
        Ok(params) => params,
        Err(error) => return failure(id, &error),
    };
    match dispatch(node, method, params).await {
        Ok(result) => success(id, result),
        Err(error) => failure(id, &error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::Node;
    use botchain_core::{Allocation, Amount, Genesis, Keypair};

    fn node() -> SharedNode {
        let genesis = Genesis {
            chain_id: "botchain-devnet".to_string(),
            timestamp_ms: 1_790_000_000_000,
            allocations: vec![Allocation {
                address: Keypair::from_secret_bytes([1u8; 32]).address(),
                amount: Amount::from_u64(1000),
            }],
        };
        Node::from_genesis(&genesis, 1000).expect("node")
    }

    #[tokio::test]
    async fn a_call_gets_a_jsonrpc_response() {
        let node = node();
        let response = handle_request(
            &node,
            json!({"jsonrpc": "2.0", "id": 7, "method": "getHealth"}),
        )
        .await;
        assert_eq!(response, json!({"jsonrpc": "2.0", "id": 7, "result": "ok"}));
    }

    #[tokio::test]
    async fn string_and_missing_ids_come_back_unchanged() {
        let node = node();
        let response = handle_request(
            &node,
            json!({"jsonrpc": "2.0", "id": "abc", "method": "getBlockHeight", "params": {}}),
        )
        .await;
        assert_eq!(response["id"], json!("abc"));
        assert_eq!(response["result"], json!(0));

        let response = handle_request(&node, json!({"method": "getBlockHeight"})).await;
        assert_eq!(response["id"], Value::Null);
        assert_eq!(response["result"], json!(0));
    }

    #[tokio::test]
    async fn broken_requests_get_error_objects() {
        let node = node();
        let response =
            handle_request(&node, json!({"jsonrpc": "2.0", "id": 1, "method": "nope"})).await;
        assert_eq!(response["error"]["code"], json!(-32601));

        let response =
            handle_request(&node, json!({"jsonrpc": "1.0", "id": 1, "method": "x"})).await;
        assert_eq!(response["error"]["code"], json!(-32600));

        let response = handle_request(&node, json!({"id": 1})).await;
        assert_eq!(response["error"]["code"], json!(-32600));

        let response = handle_request(&node, json!("hello")).await;
        assert_eq!(response["error"]["code"], json!(-32600));

        let response = handle_request(
            &node,
            json!({"jsonrpc": "2.0", "id": 1, "method": "getBlock", "params": [0]}),
        )
        .await;
        assert_eq!(response["error"]["code"], json!(-32602));
    }

    #[tokio::test]
    async fn batches_answer_in_order() {
        let node = node();
        let response = handle_request(
            &node,
            json!([
                {"jsonrpc": "2.0", "id": 1, "method": "getHealth"},
                {"jsonrpc": "2.0", "id": 2, "method": "getBlockHeight"}
            ]),
        )
        .await;
        let calls = response.as_array().expect("array");
        assert_eq!(calls[0]["result"], json!("ok"));
        assert_eq!(calls[1]["id"], json!(2));
    }
}

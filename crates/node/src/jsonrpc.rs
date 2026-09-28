//! JSON-RPC 2.0 plumbing: requests, responses, errors and named params.

use serde_json::{json, Map, Value};

/// The standard "parse error" code.
pub const PARSE_ERROR: i64 = -32700;
/// The standard "invalid request" code.
pub const INVALID_REQUEST: i64 = -32600;
/// The standard "method not found" code.
pub const METHOD_NOT_FOUND: i64 = -32601;
/// The standard "invalid params" code.
pub const INVALID_PARAMS: i64 = -32602;
/// Our generic failure code, inside the -32000..=-32099 server range.
pub const SERVER_ERROR: i64 = -32000;
/// A transaction the node refuses to accept.
pub const TX_REJECTED: i64 = -32001;

/// A JSON-RPC error object.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RpcError {
    /// The error code.
    pub code: i64,
    /// A short human readable message.
    pub message: String,
}

impl RpcError {
    /// An error with any code and message.
    pub fn new(code: i64, message: impl Into<String>) -> RpcError {
        RpcError {
            code,
            message: message.into(),
        }
    }

    /// -32601: the method does not exist.
    pub fn method_not_found(method: &str) -> RpcError {
        RpcError::new(METHOD_NOT_FOUND, format!("unknown method: {method}"))
    }

    /// -32602: the params are missing, of the wrong type or out of range.
    pub fn invalid_params(message: impl Into<String>) -> RpcError {
        RpcError::new(INVALID_PARAMS, message)
    }

    /// -32001: the transaction was refused.
    pub fn rejected(message: impl Into<String>) -> RpcError {
        RpcError::new(TX_REJECTED, message)
    }

    /// -32000: anything else that went wrong while serving the call.
    pub fn server_error(message: impl Into<String>) -> RpcError {
        RpcError::new(SERVER_ERROR, message)
    }

    /// The JSON object that goes into a response.
    pub fn to_value(&self) -> Value {
        json!({"code": self.code, "message": self.message})
    }
}

/// Builds a successful JSON-RPC response.
pub fn success(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

/// Builds a failed JSON-RPC response.
pub fn failure(id: Value, error: &RpcError) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": error.to_value()})
}

/// The named parameters of a call, with typed accessors.
///
/// A missing `params` and an explicit `null` both mean "no parameters"; an
/// array is refused, the interface only uses named parameters.
#[derive(Clone, Debug, Default)]
pub struct Params(Map<String, Value>);

impl Params {
    /// Reads the `params` member of a request.
    pub fn from_value(value: Option<&Value>) -> Result<Params, RpcError> {
        match value {
            None | Some(Value::Null) => Ok(Params(Map::new())),
            Some(Value::Object(map)) => Ok(Params(map.clone())),
            Some(_) => Err(RpcError::invalid_params(
                "params must be an object of named arguments",
            )),
        }
    }

    /// The raw value of a parameter, if it was given and is not null.
    pub fn get(&self, name: &str) -> Option<&Value> {
        match self.0.get(name) {
            None | Some(Value::Null) => None,
            Some(value) => Some(value),
        }
    }

    /// A required raw parameter.
    pub fn require(&self, name: &str) -> Result<&Value, RpcError> {
        self.get(name)
            .ok_or_else(|| RpcError::invalid_params(format!("missing parameter: {name}")))
    }

    /// A required string parameter.
    pub fn string(&self, name: &str) -> Result<&str, RpcError> {
        self.require(name)?
            .as_str()
            .ok_or_else(|| RpcError::invalid_params(format!("{name} must be a string")))
    }

    /// A required non negative integer parameter.
    pub fn u64(&self, name: &str) -> Result<u64, RpcError> {
        self.require(name)?.as_u64().ok_or_else(|| {
            RpcError::invalid_params(format!("{name} must be a non negative integer"))
        })
    }

    /// An optional integer parameter, checked against an inclusive range.
    pub fn u64_in_range(
        &self,
        name: &str,
        min: u64,
        max: u64,
        default: u64,
    ) -> Result<u64, RpcError> {
        let Some(value) = self.get(name) else {
            return Ok(default);
        };
        let number = value.as_u64().filter(|n| *n >= min && *n <= max);
        number.ok_or_else(|| {
            RpcError::invalid_params(format!("{name} must be an integer between {min} and {max}"))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(json: Value) -> Params {
        Params::from_value(Some(&json)).expect("params")
    }

    #[test]
    fn missing_params_are_empty() {
        assert!(Params::from_value(None).expect("none").get("x").is_none());
        assert!(Params::from_value(Some(&Value::Null))
            .expect("null")
            .get("x")
            .is_none());
        let error = Params::from_value(Some(&json!([1, 2]))).expect_err("array");
        assert_eq!(error.code, INVALID_PARAMS);
    }

    #[test]
    fn typed_accessors_report_clear_errors() {
        let p = params(json!({"height": 3, "address": "abc", "bad": -1}));
        assert_eq!(p.u64("height"), Ok(3));
        assert_eq!(p.string("address"), Ok("abc"));
        assert_eq!(p.u64("bad").expect_err("negative").code, INVALID_PARAMS);
        assert!(p.u64("nope").expect_err("missing").message.contains("nope"));
        assert!(p
            .string("height")
            .expect_err("not a string")
            .message
            .contains("must be a string"));
    }

    #[test]
    fn ranged_parameter_defaults_and_bounds() {
        let empty = params(json!({}));
        assert_eq!(empty.u64_in_range("limit", 1, 100, 20), Ok(20));
        let given = params(json!({"limit": 5}));
        assert_eq!(given.u64_in_range("limit", 1, 100, 20), Ok(5));
        for bad in [
            json!({"limit": 0}),
            json!({"limit": 101}),
            json!({"limit": "5"}),
        ] {
            assert_eq!(
                params(bad)
                    .u64_in_range("limit", 1, 100, 20)
                    .expect_err("out of range")
                    .code,
                INVALID_PARAMS
            );
        }
    }

    #[test]
    fn response_shapes() {
        assert_eq!(
            success(json!(1), json!("ok")),
            json!({"jsonrpc": "2.0", "id": 1, "result": "ok"})
        );
        assert_eq!(
            failure(Value::Null, &RpcError::method_not_found("nope")),
            json!({"jsonrpc": "2.0", "id": null,
                   "error": {"code": -32601, "message": "unknown method: nope"}})
        );
    }
}

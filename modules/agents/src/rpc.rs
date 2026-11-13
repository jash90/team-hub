//! JSON-RPC 2.0 as MCP uses it: one message per request, answered with one message.

use serde_json::{json, Value};

pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
pub const INTERNAL_ERROR: i64 = -32603;

#[derive(Debug, Clone, PartialEq)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
}

impl RpcError {
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    /// A request, which gets an answer.
    Call { id: Value, method: String, params: Value },
    /// A notification, or the client's answer to something: accepted without a reply.
    Notification,
    /// Not a message this server can act on; `id` is the request's when it could be read.
    Invalid { id: Value, error: RpcError },
}

fn invalid(id: Value, message: &str) -> Incoming {
    Incoming::Invalid { id, error: RpcError::new(INVALID_REQUEST, format!("Invalid request: {message}")) }
}

pub fn classify(message: Value) -> Incoming {
    let Value::Object(mut message) = message else {
        let reason = if message.is_array() { "batches are not supported, send one message per request" } else { "a message is a JSON object" };

        return invalid(Value::Null, reason);
    };
    let id = message.remove("id");
    let usable_id = id.clone().filter(|id| id.is_string() || id.is_number()).unwrap_or(Value::Null);

    if message.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return invalid(usable_id, "\"jsonrpc\" must be \"2.0\"");
    }

    let method = match message.remove("method") {
        Some(Value::String(method)) => method,
        Some(_) => return invalid(usable_id, "\"method\" must be a string"),
        None if id.is_some() && (message.contains_key("result") || message.contains_key("error")) => return Incoming::Notification,
        None => return invalid(usable_id, "\"method\" is missing"),
    };

    match id {
        None => Incoming::Notification,
        Some(id) if id.is_string() || id.is_number() => Incoming::Call { id, method, params: message.remove("params").unwrap_or(Value::Null) },
        Some(_) => invalid(Value::Null, "\"id\" must be a string or a number"),
    }
}

pub fn success(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

pub fn failure(id: Value, error: &RpcError) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": error.code, "message": error.message } })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(incoming: Incoming) -> Option<i64> {
        match incoming {
            Incoming::Invalid { error, .. } => Some(error.code),
            _ => None,
        }
    }

    #[test]
    fn a_message_with_an_id_is_a_call_and_one_without_is_a_notification() {
        let call = classify(json!({ "jsonrpc": "2.0", "id": 7, "method": "tools/list" }));
        let named = classify(json!({ "jsonrpc": "2.0", "id": "a", "method": "tools/call", "params": { "name": "x" } }));

        assert_eq!(call, Incoming::Call { id: json!(7), method: "tools/list".to_string(), params: Value::Null });
        assert_eq!(named, Incoming::Call { id: json!("a"), method: "tools/call".to_string(), params: json!({ "name": "x" }) });
        assert_eq!(classify(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })), Incoming::Notification);
        assert_eq!(classify(json!({ "jsonrpc": "2.0", "id": 3, "result": {} })), Incoming::Notification);
    }

    #[test]
    fn anything_else_is_an_invalid_request_that_keeps_the_id_when_it_has_one() {
        assert_eq!(code(classify(json!([{ "jsonrpc": "2.0", "id": 1, "method": "ping" }]))), Some(INVALID_REQUEST));
        assert_eq!(code(classify(json!("ping"))), Some(INVALID_REQUEST));
        assert_eq!(code(classify(json!({ "id": 1, "method": "ping" }))), Some(INVALID_REQUEST));
        assert_eq!(code(classify(json!({ "jsonrpc": "2.0", "id": 1 }))), Some(INVALID_REQUEST));
        assert_eq!(code(classify(json!({ "jsonrpc": "2.0", "id": null, "method": "ping" }))), Some(INVALID_REQUEST));
        assert!(matches!(classify(json!({ "jsonrpc": "1.0", "id": 5, "method": "ping" })), Incoming::Invalid { id, .. } if id == json!(5)));
    }
}

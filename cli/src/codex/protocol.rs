use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const MAX_JSON_LINE_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_ASSISTANT_BYTES: usize = 64 * 1024;

#[derive(Serialize)]
pub struct Request<'a> {
    pub id: u64,
    pub method: &'a str,
    pub params: Value,
}

#[derive(Serialize)]
pub struct Notification<'a> {
    pub method: &'a str,
    pub params: Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Response {
    id: u64,
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    error: Option<RpcError>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RpcError {
    code: i64,
    message: String,
    #[serde(default)]
    data: Option<Value>,
}

pub enum Incoming {
    Response {
        id: u64,
        result: Result<Value, String>,
    },
    Notification {
        method: String,
        params: Value,
    },
    ServerRequest {
        id: Value,
        method: String,
    },
}

pub fn decode(line: &[u8]) -> Result<Incoming, String> {
    if line.len() > MAX_JSON_LINE_BYTES {
        return Err("Codex protocol line exceeds 2 MiB".into());
    }
    let value: Value =
        serde_json::from_slice(line).map_err(|_| "Codex emitted malformed JSON".to_string())?;
    let object = value.as_object().ok_or("Codex message is not an object")?;
    if object.contains_key("jsonrpc") {
        return Err("unsupported Codex JSON-RPC envelope".into());
    }
    if object.contains_key("id") && (object.contains_key("result") || object.contains_key("error"))
    {
        let response: Response = serde_json::from_value(value)
            .map_err(|_| "malformed Codex response envelope".to_string())?;
        if response.result.is_some() == response.error.is_some() {
            return Err("Codex response must contain exactly one of result or error".into());
        }
        return Ok(Incoming::Response {
            id: response.id,
            result: response.result.ok_or_else(|| {
                let error = response.error.expect("validated error");
                let _ = error.data;
                format!("Codex error {}: {}", error.code, error.message)
            }),
        });
    }
    let method = object
        .get("method")
        .and_then(Value::as_str)
        .ok_or("Codex message has no method")?
        .to_string();
    if let Some(id) = object.get("id") {
        return Ok(Incoming::ServerRequest {
            id: id.clone(),
            method,
        });
    }
    let params = object.get("params").cloned().unwrap_or_else(|| json!({}));
    if object.keys().any(|key| key != "method" && key != "params") {
        return Err("malformed Codex notification envelope".into());
    }
    Ok(Incoming::Notification { method, params })
}

pub fn request(id: u64, method: &str, params: Value) -> Result<Vec<u8>, String> {
    assert!(id > 0);
    let mut bytes = serde_json::to_vec(&Request { id, method, params })
        .map_err(|_| "cannot encode Codex request".to_string())?;
    bytes.push(b'\n');
    assert!(bytes.len() <= MAX_JSON_LINE_BYTES);
    Ok(bytes)
}

pub fn notification(method: &str, params: Value) -> Result<Vec<u8>, String> {
    let mut bytes = serde_json::to_vec(&Notification { method, params })
        .map_err(|_| "cannot encode Codex notification".to_string())?;
    bytes.push(b'\n');
    assert!(bytes.len() <= MAX_JSON_LINE_BYTES);
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn omitted_jsonrpc_and_strict_pending_response_envelopes() {
        let encoded = request(1, "account/read", json!({"refreshToken": false})).unwrap();
        assert!(!String::from_utf8(encoded).unwrap().contains("jsonrpc"));
        assert!(matches!(
            decode(br#"{"id":1,"result":{}}"#).unwrap(),
            Incoming::Response { id: 1, .. }
        ));
        assert!(decode(br#"{"jsonrpc":"2.0","id":1,"result":{}}"#).is_err());
        assert!(decode(br#"{"id":1,"result":{},"extra":1}"#).is_err());
    }

    #[test]
    fn unknown_notifications_are_typed_for_caller_to_ignore() {
        assert!(matches!(
            decode(br#"{"method":"future/event","params":{"x":1}}"#).unwrap(),
            Incoming::Notification { .. }
        ));
    }
}

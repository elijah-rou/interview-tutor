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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub enum RequestId {
    Number(u64),
    String(String),
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ServerRequest {
    id: RequestId,
    method: String,
    params: Value,
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
        id: RequestId,
        method: String,
        params: Value,
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
    if object.contains_key("id") {
        let request: ServerRequest = serde_json::from_value(value)
            .map_err(|_| "malformed Codex server request envelope".to_string())?;
        return Ok(Incoming::ServerRequest {
            id: request.id,
            method: request.method,
            params: request.params,
        });
    }
    let method = object
        .get("method")
        .and_then(Value::as_str)
        .ok_or("Codex message has no method")?
        .to_string();
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
    if bytes.len() > MAX_JSON_LINE_BYTES {
        return Err("Codex request exceeds 2 MiB".into());
    }
    Ok(bytes)
}

pub fn notification(method: &str, params: Value) -> Result<Vec<u8>, String> {
    let mut bytes = serde_json::to_vec(&Notification { method, params })
        .map_err(|_| "cannot encode Codex notification".to_string())?;
    bytes.push(b'\n');
    if bytes.len() > MAX_JSON_LINE_BYTES {
        return Err("Codex notification exceeds 2 MiB".into());
    }
    Ok(bytes)
}

pub fn server_response(id: &RequestId, result: Value) -> Result<Vec<u8>, String> {
    let mut bytes = serde_json::to_vec(&json!({"id":id,"result":result}))
        .map_err(|_| "cannot encode Codex server response".to_string())?;
    bytes.push(b'\n');
    if bytes.len() > MAX_JSON_LINE_BYTES {
        return Err("Codex server response exceeds 2 MiB".into());
    }
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
    fn server_requests_retain_typed_ids_and_params() {
        let incoming = decode(
            br#"{"id":"approval-1","method":"item/fileChange/requestApproval","params":{"turnId":"turn-1"}}"#,
        )
        .unwrap();
        let Incoming::ServerRequest { id, method, params } = incoming else {
            panic!("expected server request")
        };
        assert_eq!(id, RequestId::String("approval-1".into()));
        assert_eq!(method, "item/fileChange/requestApproval");
        assert_eq!(params["turnId"], "turn-1");
        assert!(decode(br#"{"id":{},"method":"x","params":{}}"#).is_err());
        assert!(decode(br#"{"id":1,"method":"x","params":{},"extra":1}"#).is_err());
    }

    #[test]
    fn unknown_notifications_are_typed_for_caller_to_ignore() {
        assert!(matches!(
            decode(br#"{"method":"future/event","params":{"x":1}}"#).unwrap(),
            Incoming::Notification { .. }
        ));
    }
}

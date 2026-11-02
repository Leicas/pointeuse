//! Odoo JSON-RPC transport — the only wire protocol this app speaks to Odoo.
//!
//! Odoo 19 serves the classic `POST /jsonrpc` endpoint (service/method/args
//! envelope), which maps 1:1 onto the former XML-RPC services:
//!
//! * service `common` → `authenticate` / `version`
//! * service `object` → `execute_kw(db, uid, password, model, method, args, kwargs)`
//! * service `db`     → `list`
//!
//! Both XML-RPC and `/jsonrpc` are deprecated in Odoo 19 in favour of the
//! JSON-2 API (`/json/2/<model>/<method>`, bearer API keys). Keeping the
//! transport in this one module makes that later move a single-file change.

use serde_json::{json, Map, Number, Value};
use std::collections::HashMap;

use crate::error::{AppError, AppResult};

/// Path of Odoo's JSON-RPC endpoint, relative to the instance URL.
pub const JSONRPC_PATH: &str = "/jsonrpc";

// ---------------------------------------------------------------------------
// OdooValue
// ---------------------------------------------------------------------------

/// A value exchanged with Odoo. `Nil` is JSON `null` (Python `None`).
#[derive(Debug, Clone, PartialEq)]
pub enum OdooValue {
    Int(i64),
    Bool(bool),
    Double(f64),
    String(String),
    Array(Vec<OdooValue>),
    Struct(HashMap<String, OdooValue>),
    Nil,
}

impl OdooValue {
    // -- convenience accessors ------------------------------------------------

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            OdooValue::Int(v) => Some(*v),
            OdooValue::Double(v) => Some(*v as i64),
            OdooValue::Bool(b) => Some(if *b { 1 } else { 0 }),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            OdooValue::Double(v) => Some(*v),
            OdooValue::Int(v) => Some(*v as f64),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            OdooValue::String(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            OdooValue::Bool(b) => Some(*b),
            OdooValue::Int(v) => Some(*v != 0),
            _ => None,
        }
    }

    #[allow(dead_code)]
    pub fn as_array(&self) -> Option<&Vec<OdooValue>> {
        match self {
            OdooValue::Array(a) => Some(a),
            _ => None,
        }
    }

    pub fn as_struct(&self) -> Option<&HashMap<String, OdooValue>> {
        match self {
            OdooValue::Struct(m) => Some(m),
            _ => None,
        }
    }

    /// Odoo quirk: a many2one field is either `[id, "name"]` or `false`.
    /// This helper returns `None` for both `Nil` and `Bool(false)`.
    pub fn as_many2one(&self) -> Option<(i64, String)> {
        match self {
            OdooValue::Bool(false) | OdooValue::Nil => None,
            OdooValue::Array(arr) if arr.len() == 2 => {
                let id = arr[0].as_i64()?;
                let name = arr[1].as_str()?.to_string();
                Some((id, name))
            }
            _ => None,
        }
    }

    // -- JSON conversion ------------------------------------------------------

    pub fn to_json(&self) -> Value {
        match self {
            OdooValue::Int(i) => Value::Number((*i).into()),
            OdooValue::Bool(b) => Value::Bool(*b),
            // NaN / infinity have no JSON form; Odoo never expects them.
            OdooValue::Double(d) => Number::from_f64(*d).map(Value::Number).unwrap_or(Value::Null),
            OdooValue::String(s) => Value::String(s.clone()),
            OdooValue::Array(a) => Value::Array(a.iter().map(OdooValue::to_json).collect()),
            OdooValue::Struct(m) => {
                Value::Object(m.iter().map(|(k, v)| (k.clone(), v.to_json())).collect::<Map<_, _>>())
            }
            OdooValue::Nil => Value::Null,
        }
    }

    pub fn from_json(v: &Value) -> OdooValue {
        match v {
            Value::Null => OdooValue::Nil,
            Value::Bool(b) => OdooValue::Bool(*b),
            Value::Number(n) => match n.as_i64() {
                Some(i) => OdooValue::Int(i),
                None => OdooValue::Double(n.as_f64().unwrap_or(0.0)),
            },
            Value::String(s) => OdooValue::String(s.clone()),
            Value::Array(a) => OdooValue::Array(a.iter().map(OdooValue::from_json).collect()),
            Value::Object(m) => {
                OdooValue::Struct(m.iter().map(|(k, v)| (k.clone(), OdooValue::from_json(v))).collect())
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Envelope
// ---------------------------------------------------------------------------

fn build_request(service: &str, method: &str, args: &[OdooValue]) -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "call",
        "id": 1,
        "params": {
            "service": service,
            "method": method,
            "args": args.iter().map(OdooValue::to_json).collect::<Vec<_>>(),
        }
    })
}

/// Parse a `/jsonrpc` response body. Returns `result`; an `error` member
/// becomes `AppError::Odoo` carrying Odoo's own message and exception class.
fn parse_response(text: &str) -> AppResult<OdooValue> {
    let parsed: Value =
        serde_json::from_str(text).map_err(|_| AppError::Odoo("Invalid JSON-RPC response".into()))?;

    if let Some(err) = parsed.get("error").filter(|e| !e.is_null()) {
        let data = err.get("data");
        let message = data
            .and_then(|d| d.get("message"))
            .and_then(Value::as_str)
            .filter(|m| !m.trim().is_empty())
            .or_else(|| err.get("message").and_then(Value::as_str))
            .unwrap_or("JSON-RPC fault");
        return Err(match data.and_then(|d| d.get("name")).and_then(Value::as_str) {
            Some(name) => AppError::Odoo(format!("Odoo fault ({name}): {message}")),
            None => AppError::Odoo(format!("Odoo fault: {message}")),
        });
    }

    parsed
        .get("result")
        .map(OdooValue::from_json)
        .ok_or_else(|| AppError::Odoo("Invalid JSON-RPC response".into()))
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Full error chain ("error sending request -> dns error -> ..."): reqwest's
/// top-level Display hides the actual cause, which is the part we need when
/// diagnosing connection failures in the field.
fn error_chain(e: &dyn std::error::Error) -> String {
    let mut chain = e.to_string();
    let mut src = e.source();
    while let Some(s) = src {
        chain.push_str(" -> ");
        chain.push_str(&s.to_string());
        src = s.source();
    }
    chain
}

/// Perform one JSON-RPC call against an Odoo instance.
///
/// * `client`  – shared reqwest client
/// * `url`     – base URL of the Odoo instance, e.g. `https://mycompany.odoo.com`
/// * `service` – `common`, `object` or `db`
/// * `method`  – service method, e.g. `authenticate` or `execute_kw`
/// * `args`    – positional parameters
pub async fn call_jsonrpc(
    client: &reqwest::Client,
    url: &str,
    service: &str,
    method: &str,
    args: Vec<OdooValue>,
) -> AppResult<OdooValue> {
    let full_url = format!("{}{}", url.trim_end_matches('/'), JSONRPC_PATH);
    let body = build_request(service, method, &args);

    log::debug!("JSON-RPC POST {full_url} service={service} method={method}");

    let resp = client
        .post(&full_url)
        .header("Accept", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| {
            let chain = error_chain(&e);
            log::error!("JSON-RPC send failed for {full_url}: {chain}");
            AppError::Odoo(format!("Request to {full_url} failed: {chain}"))
        })?;

    let status = resp.status();
    let text = resp.text().await.map_err(|e| {
        let chain = error_chain(&e);
        log::error!("JSON-RPC body read failed for {full_url}: {chain}");
        AppError::Odoo(format!("Reading response from {full_url} failed: {chain}"))
    })?;

    log::debug!("JSON-RPC response ({} bytes) for {service}.{method}", text.len());
    log::trace!("JSON-RPC response body:\n{text}");

    // Odoo answers faults with HTTP 200 + an `error` member, so try to parse
    // first; a proxy or login page answers with HTML and a 4xx/5xx.
    match parse_response(&text) {
        Ok(v) => Ok(v),
        Err(e) if !status.is_success() && text.trim_start().starts_with('{') => Err(e),
        Err(_) if !status.is_success() => Err(AppError::Odoo(format!(
            "HTTP {status} from {full_url}: {}",
            &text[..text.len().min(200)]
        ))),
        Err(e) => Err(e),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_request_envelope() {
        let req = build_request(
            "common",
            "authenticate",
            &[
                OdooValue::String("mydb".into()),
                OdooValue::String("admin".into()),
                OdooValue::String("secret".into()),
                OdooValue::Struct(HashMap::new()),
            ],
        );
        assert_eq!(req["jsonrpc"], "2.0");
        assert_eq!(req["method"], "call");
        assert_eq!(req["params"]["service"], "common");
        assert_eq!(req["params"]["method"], "authenticate");
        assert_eq!(req["params"]["args"][0], "mydb");
        assert_eq!(req["params"]["args"][3], json!({}));
    }

    #[test]
    fn test_parse_int_result() {
        let val = parse_response(r#"{"jsonrpc":"2.0","id":1,"result":42}"#).unwrap();
        assert_eq!(val.as_i64(), Some(42));
    }

    #[test]
    fn test_parse_false_result_is_bool() {
        // Odoo returns false when authentication fails
        let val = parse_response(r#"{"jsonrpc":"2.0","id":1,"result":false}"#).unwrap();
        assert_eq!(val, OdooValue::Bool(false));
        assert_eq!(val.as_i64(), Some(0));
    }

    #[test]
    fn test_parse_record_with_many2one_and_null() {
        let val = parse_response(
            r#"{"jsonrpc":"2.0","id":1,"result":[{"id":7,"name":"T","project_id":[3,"P"],"date_deadline":false,"x":null,"hours":8.0}]}"#,
        )
        .unwrap();
        let rec = val.as_array().unwrap()[0].as_struct().unwrap();
        assert_eq!(rec["id"].as_i64(), Some(7));
        assert_eq!(rec["project_id"].as_many2one(), Some((3, "P".into())));
        assert_eq!(rec["date_deadline"].as_many2one(), None);
        assert_eq!(rec["x"], OdooValue::Nil);
        assert_eq!(rec["hours"], OdooValue::Double(8.0));
    }

    #[test]
    fn test_parse_fault() {
        let err = parse_response(
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":200,"message":"Odoo Server Error","data":{"name":"odoo.exceptions.AccessDenied","message":"Access Denied"}}}"#,
        )
        .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("Access Denied"), "got: {msg}");
        assert!(msg.contains("odoo.exceptions.AccessDenied"), "got: {msg}");
    }

    #[test]
    fn test_parse_rejects_non_envelope() {
        assert!(parse_response("<html>login</html>").is_err());
        assert!(parse_response(r#"{"jsonrpc":"2.0","id":1}"#).is_err());
    }

    #[test]
    fn test_json_roundtrip_keeps_int_and_double() {
        let v = OdooValue::Array(vec![OdooValue::Int(5), OdooValue::Double(1.5), OdooValue::Nil]);
        assert_eq!(OdooValue::from_json(&v.to_json()), v);
    }

    #[test]
    fn test_parse_boolean_false_as_empty_many2one() {
        // Odoo returns False for empty many2one fields
        assert_eq!(OdooValue::Bool(false).as_many2one(), None);
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
//! hello, isMaster, ping, buildInfo commands.

use crate::bson::Document;
use serde_json::Value;

pub async fn hello(_cmd_doc: &Document) -> Result<Document, String> {
    let mut resp = Document::new();
    resp.insert("ismaster".to_string(), Value::Bool(true));
    resp.insert("maxBsonObjectSize".to_string(), Value::Number(16777216.into()));
    resp.insert("maxMessageSizeBytes".to_string(), Value::Number(48000000.into()));
    resp.insert("minWireVersion".to_string(), Value::Number(0.into()));
    resp.insert("maxWireVersion".to_string(), Value::Number(17.into()));
    resp.insert("logicalSessionTimeoutMinutes".to_string(), Value::Number(30.into()));
    resp.insert("ok".to_string(), Value::Number(1.into()));
    Ok(resp)
}

pub async fn ping() -> Result<Document, String> {
    let mut resp = Document::new();
    resp.insert("ok".to_string(), Value::Number(1.into()));
    Ok(resp)
}

pub async fn build_info() -> Result<Document, String> {
    let mut resp = Document::new();
    resp.insert("version".to_string(), Value::String("6.0.0".to_string()));
    resp.insert("gitVersion".to_string(), Value::String("cave".to_string()));
    resp.insert("modules".to_string(), Value::Array(vec![]));
    resp.insert("ok".to_string(), Value::Number(1.into()));
    Ok(resp)
}

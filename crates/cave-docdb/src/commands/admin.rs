// SPDX-License-Identifier: AGPL-3.0-or-later
//! Admin commands: endSessions, currentOp, serverStatus.

use crate::bson::Document;
use serde_json::Value;

pub async fn end_sessions() -> Result<Document, String> {
    let mut resp = Document::new();
    resp.insert("ok".to_string(), Value::Number(1.into()));
    Ok(resp)
}

pub async fn current_op() -> Result<Document, String> {
    let mut resp = Document::new();
    resp.insert("inprog".to_string(), Value::Array(vec![]));
    resp.insert("ok".to_string(), Value::Number(1.into()));
    Ok(resp)
}

pub async fn server_status() -> Result<Document, String> {
    let mut resp = Document::new();
    resp.insert("host".to_string(), Value::String("localhost".to_string()));
    resp.insert("version".to_string(), Value::String("6.0.0".to_string()));
    resp.insert("uptime".to_string(), Value::Number(0.into()));
    resp.insert(
        "connections".to_string(),
        Value::Object(
            vec![
                ("current".to_string(), Value::Number(1.into())),
                ("available".to_string(), Value::Number(1000.into())),
            ]
            .into_iter()
            .collect(),
        ),
    );
    resp.insert("ok".to_string(), Value::Number(1.into()));
    Ok(resp)
}

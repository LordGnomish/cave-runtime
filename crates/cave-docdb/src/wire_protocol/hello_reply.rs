// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: FerretDB/FerretDB@27653503a28b0d2f2a09d7105887edd3a8af1060 internal/handler/msg_hello.go
//! `hello` / `isMaster` reply builder.
//!
//! Returns the canonical fields a MongoDB driver consumes during the
//! initial handshake — wire-version range, max BSON size, the standard
//! `ok: 1`, and the topology hint. Single-node deployments report
//! `isWritablePrimary: true` and an empty hosts list.

use crate::bson::Document;
use serde_json::Value;

/// Compiled reply.
#[derive(Debug, Clone)]
pub struct HelloReply {
    pub body: Document,
}

/// MongoDB driver-handshake constants (publicly documented).
pub const MAX_BSON_OBJECT_SIZE: i64 = 16 * 1024 * 1024;
pub const MAX_WRITE_BATCH_SIZE: i64 = 100_000;
pub const MAX_MESSAGE_SIZE_BYTES: i64 = 48_000_000;
pub const MIN_WIRE_VERSION: i64 = 0;
pub const MAX_WIRE_VERSION: i64 = 21;
pub const LOGICAL_SESSION_TIMEOUT_MINUTES: i64 = 30;

/// Build the standard hello reply. `set_name` is `Some(name)` if the
/// node is part of a replica set, `None` for stand-alone deployments.
pub fn hello_reply(
    is_primary: bool,
    set_name: Option<&str>,
    secondary_hosts: &[String],
) -> HelloReply {
    let mut body = Document::new();
    body.insert("isWritablePrimary".to_string(), Value::Bool(is_primary));
    // Legacy alias for older drivers.
    body.insert("ismaster".to_string(), Value::Bool(is_primary));
    body.insert("maxBsonObjectSize".to_string(), Value::Number(MAX_BSON_OBJECT_SIZE.into()));
    body.insert("maxMessageSizeBytes".to_string(), Value::Number(MAX_MESSAGE_SIZE_BYTES.into()));
    body.insert("maxWriteBatchSize".to_string(), Value::Number(MAX_WRITE_BATCH_SIZE.into()));
    body.insert("localTime".to_string(), Value::Number(now_unix_seconds().into()));
    body.insert(
        "logicalSessionTimeoutMinutes".to_string(),
        Value::Number(LOGICAL_SESSION_TIMEOUT_MINUTES.into()),
    );
    body.insert("connectionId".to_string(), Value::Number(0.into()));
    body.insert("minWireVersion".to_string(), Value::Number(MIN_WIRE_VERSION.into()));
    body.insert("maxWireVersion".to_string(), Value::Number(MAX_WIRE_VERSION.into()));
    body.insert("readOnly".to_string(), Value::Bool(false));
    if let Some(name) = set_name {
        body.insert("setName".to_string(), Value::String(name.to_string()));
        body.insert(
            "hosts".to_string(),
            Value::Array(secondary_hosts.iter().cloned().map(Value::String).collect()),
        );
    }
    body.insert("ok".to_string(), Value::Number(1.into()));
    HelloReply { body }
}

fn now_unix_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standalone_reports_primary_true_no_set_name() {
        let r = hello_reply(true, None, &[]);
        assert_eq!(r.body.get("isWritablePrimary"), Some(&Value::Bool(true)));
        assert!(!r.body.contains_key("setName"));
    }

    #[test]
    fn secondary_reports_primary_false() {
        let r = hello_reply(false, Some("rs0"), &["a:27017".into(), "b:27017".into()]);
        assert_eq!(r.body.get("isWritablePrimary"), Some(&Value::Bool(false)));
        assert_eq!(r.body.get("setName"), Some(&Value::String("rs0".into())));
        let hosts = r.body.get("hosts").unwrap().as_array().unwrap();
        assert_eq!(hosts.len(), 2);
    }

    #[test]
    fn legacy_ismaster_alias_present() {
        let r = hello_reply(true, None, &[]);
        assert_eq!(r.body.get("ismaster"), Some(&Value::Bool(true)));
    }

    #[test]
    fn ok_field_is_numeric_one() {
        let r = hello_reply(true, None, &[]);
        assert_eq!(r.body.get("ok"), Some(&Value::Number(1.into())));
    }

    #[test]
    fn wire_versions_match_spec_constants() {
        let r = hello_reply(true, None, &[]);
        assert_eq!(r.body.get("minWireVersion").unwrap().as_i64().unwrap(), 0);
        assert_eq!(r.body.get("maxWireVersion").unwrap().as_i64().unwrap(), 21);
    }

    #[test]
    fn max_bson_size_is_16mib() {
        let r = hello_reply(true, None, &[]);
        assert_eq!(
            r.body.get("maxBsonObjectSize").unwrap().as_i64().unwrap(),
            16 * 1024 * 1024
        );
    }
}

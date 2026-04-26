//! Audit log JSON formatting — parity tests against openbao v2.5.3.
//!
//! Upstream package: `audit/`. cave-vault wires the audit pipeline through
//! `core::audit::AuditLogger` which mirrors openbao's `AuditFormatter` with
//! HMAC-based redaction of client_token and accessor.

use cave_vault::core::audit::{AuditAuth, AuditBackend, AuditBackendType, AuditEntry, AuditLogger, AuditRequest};
use std::collections::HashMap;

fn dummy_logger() -> AuditLogger {
    AuditLogger::new(b"test-hmac-key-32-bytes-aaaaaaaaaa".to_vec())
}

fn dummy_entry(token: &str) -> AuditEntry {
    AuditEntry {
        time: "2026-04-26T10:00:00Z".into(),
        audit_type: "request".into(),
        request: AuditRequest {
            id: "req-1".into(),
            operation: "read".into(),
            mount_type: "kv".into(),
            path: "secret/data/foo".into(),
            remote_address: "127.0.0.1".into(),
        },
        auth: Some(AuditAuth {
            client_token: token.into(),
            accessor: "acc-1".into(),
            display_name: "alice".into(),
            policies: vec!["default".into()],
            token_type: "service".into(),
        }),
        error: None,
    }
}

/// Cite: openbao `audit/format.go:71` (HashAuth) and the audit pipeline
/// invariant — the raw `client_token` must never be persisted; it is
/// always replaced with an HMAC-SHA256 of the value plus the audit salt.
#[test]
fn log_replaces_client_token_with_hmac() {
    let logger = dummy_logger();
    logger.log(dummy_entry("hvs.SECRET.PLAINTEXT"));

    let entries = logger.recent_entries(10);
    assert_eq!(entries.len(), 1);
    let auth = entries[0].auth.as_ref().unwrap();
    assert_ne!(auth.client_token, "hvs.SECRET.PLAINTEXT",
        "raw token MUST never reach the audit log");
    assert_eq!(auth.client_token.len(), 64, "SHA-256 hex is 64 chars");
}

/// Cite: openbao `audit/format.go:34` (AuditFormatter) — identical inputs
/// hashed with the same salt produce identical HMACs (deterministic
/// redaction), enabling cross-event correlation without leaking secrets.
#[test]
fn hmac_is_deterministic_for_same_input() {
    let logger = dummy_logger();
    let h1 = logger.hmac_value("identical-token");
    let h2 = logger.hmac_value("identical-token");
    let h3 = logger.hmac_value("different-token");
    assert_eq!(h1, h2, "deterministic: same value → same HMAC");
    assert_ne!(h1, h3, "different values → different HMACs");
}

/// Cite: openbao `audit/format_json.go:22` (JSONFormatWriter.WriteRequest)
/// — the audit envelope must be JSON-serialisable round-trip with all the
/// canonical fields populated.
#[test]
fn audit_entry_round_trips_through_json() {
    let logger = dummy_logger();
    let mut e = dummy_entry("plain");
    // Hash before serialising, just like the pipeline does.
    if let Some(a) = e.auth.as_mut() {
        a.client_token = logger.hmac_value(&a.client_token);
        a.accessor = logger.hmac_value(&a.accessor);
    }
    let json = serde_json::to_string(&e).expect("serialise");
    let back: AuditEntry = serde_json::from_str(&json).expect("deserialise");
    assert_eq!(back.audit_type, "request");
    assert_eq!(back.request.path, "secret/data/foo");
    assert_eq!(back.auth.unwrap().policies, vec!["default"]);
}

/// Cite: openbao audit backend management (`builtin/audit/file/backend.go`,
/// `builtin/audit/syslog/backend.go`, `builtin/audit/socket/backend.go`)
/// — multiple backends can be enabled simultaneously and each can be
/// disabled independently.
#[test]
fn audit_backends_can_be_enabled_and_disabled_independently() {
    let logger = dummy_logger();
    logger.enable("file/", AuditBackend {
        path: "file/".into(),
        backend_type: AuditBackendType::File,
        description: "primary".into(),
        options: HashMap::new(),
        local: false,
        seal_wrap: false,
    });
    logger.enable("syslog/", AuditBackend {
        path: "syslog/".into(),
        backend_type: AuditBackendType::Syslog,
        description: "secondary".into(),
        options: HashMap::new(),
        local: false,
        seal_wrap: false,
    });
    assert_eq!(logger.list_backends().len(), 2);

    assert!(logger.disable("file/"));
    assert!(!logger.disable("file/"), "second disable is idempotent");
    assert_eq!(logger.list_backends().len(), 1);
}

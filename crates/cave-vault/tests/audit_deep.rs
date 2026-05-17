// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! deeper-001: Audit log — file backend write, syslog format, signed
//! envelope, multi-backend fan-out. Pinned to openbao v2.5.3.

use cave_vault::core::audit::{
    AuditAuth, AuditBackend, AuditBackendType, AuditEntry, AuditLogger, AuditRequest,
    SignedAuditEnvelope,
};
use std::collections::HashMap;

const TENANT: &str = "tenant-acme-prod";

fn logger() -> AuditLogger {
    AuditLogger::new(b"deeper-001-hmac-key-32-bytes-aaaa".to_vec())
}

fn entry(token: &str, op: &str, path: &str) -> AuditEntry {
    AuditEntry {
        time: "2026-04-26T10:00:00Z".into(),
        audit_type: "request".into(),
        request: AuditRequest {
            id: format!("req-{}", uuid::Uuid::new_v4().simple()),
            operation: op.into(),
            mount_type: "kv".into(),
            path: path.into(),
            remote_address: "10.0.0.1".into(),
        },
        auth: Some(AuditAuth {
            client_token: token.into(),
            accessor: "acc-deep".into(),
            display_name: format!("alice@{}", TENANT),
            policies: vec!["default".into(), format!("{}-policy", TENANT)],
            token_type: "service".into(),
        }),
        error: None,
    }
}

/// Cite: openbao `builtin/audit/file/backend.go::LogRequest` — the file
/// backend appends a JSON line per request to `options["file_path"]`,
/// creating the parent directory if missing.
#[test]
fn file_backend_writes_json_line_to_configured_path() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nested").join("audit.log");
    let log = logger();
    let mut options = HashMap::new();
    options.insert("file_path".into(), path.to_string_lossy().into_owned());
    log.enable("file/", AuditBackend {
        path: "file/".into(),
        backend_type: AuditBackendType::File,
        description: "tenant audit log".into(),
        options,
        local: false,
        seal_wrap: false,
    });

    log.log(entry("hvs.SECRET", "read", &format!("{}/secret/data/foo", TENANT)));
    log.log(entry("hvs.SECRET", "update", &format!("{}/secret/data/foo", TENANT)));

    let contents = std::fs::read_to_string(&path).expect("file written");
    let lines: Vec<&str> = contents.lines().collect();
    assert_eq!(lines.len(), 2, "one JSON envelope per log call");
    for line in &lines {
        let parsed: serde_json::Value = serde_json::from_str(line).expect("valid JSON");
        // Token MUST be HMAC-redacted in the persisted envelope.
        assert_ne!(parsed["auth"]["client_token"], "hvs.SECRET");
        assert!(parsed["auth"]["client_token"].as_str().unwrap().len() == 64);
        assert_eq!(parsed["request"]["mount_type"], "kv");
    }
}

/// Cite: openbao `builtin/audit/syslog/backend.go` — the syslog backend
/// formats each entry as `<priority> <tag>: <json>` where priority is
/// `facility * 8 + severity` (LOCAL0 + INFO = 134 in the canonical map).
#[test]
fn syslog_backend_formats_priority_and_tag_correctly() {
    let mut options = HashMap::new();
    options.insert("facility".into(), "LOCAL0".into());
    options.insert("tag".into(), "vault".into());
    let backend = AuditBackend {
        path: "syslog/".into(),
        backend_type: AuditBackendType::Syslog,
        description: "syslog audit".into(),
        options,
        local: false,
        seal_wrap: false,
    };
    let line = backend.syslog_format(r#"{"audit_type":"request"}"#).unwrap();
    assert!(line.starts_with("<134> vault: "), "default LOCAL0 priority + tag");
    assert!(line.contains(r#"{"audit_type":"request"}"#));

    // LOCAL5 + custom tag
    let mut options = HashMap::new();
    options.insert("facility".into(), "LOCAL5".into());
    options.insert("tag".into(), format!("{}-vault", TENANT));
    let backend = AuditBackend {
        path: "syslog/".into(),
        backend_type: AuditBackendType::Syslog,
        description: "tenant syslog".into(),
        options,
        local: false,
        seal_wrap: false,
    };
    let line = backend.syslog_format(r#"{"x":1}"#).unwrap();
    assert!(line.starts_with(&format!("<174> {}-vault: ", TENANT)),
        "LOCAL5 priority = 174");

    // Non-syslog backend ⇒ formatter returns None.
    let bad = AuditBackend {
        path: "file/".into(),
        backend_type: AuditBackendType::File,
        description: "".into(),
        options: HashMap::new(),
        local: false,
        seal_wrap: false,
    };
    assert!(bad.syslog_format("anything").is_none());
}

/// Cite: openbao `audit/format.go:34` (AuditFormatter) + the integrator
/// pattern for signed forwarders — `signed_envelope` produces a JSON
/// payload + an HMAC signature; `verify_envelope` reproduces the HMAC
/// and compares for equality.
#[test]
fn signed_envelope_round_trips_and_detects_tampering() {
    let log = logger();
    let env = log.signed_envelope(&entry(
        "hvs.SECRET", "read", &format!("{}/secret/data/foo", TENANT),
    ));
    assert!(!env.json.is_empty());
    assert_eq!(env.signature.len(), 64, "SHA-256 HMAC hex");
    assert!(log.verify_envelope(&env), "freshly signed envelope verifies");

    // Tamper with the JSON body — verification fails.
    let tampered = SignedAuditEnvelope {
        json: env.json.replace("read", "delete"),
        signature: env.signature.clone(),
    };
    assert!(!log.verify_envelope(&tampered),
        "tampered body invalidates the signature");

    // Tamper with the signature — verification fails.
    let mut bad_sig = env.signature.clone();
    bad_sig.replace_range(0..2, "00");
    let tampered = SignedAuditEnvelope { json: env.json.clone(), signature: bad_sig };
    assert!(!log.verify_envelope(&tampered));
}

/// Cite: openbao `audit/audit.go::broker.LogRequest` — the broker fans
/// out one log call to every enabled backend before allowing the request
/// to proceed. cave's logger does the same: enabling N backends ⇒ each
/// receives the same envelope.
#[test]
fn multi_backend_fan_out_writes_to_each_enabled_backend() {
    let dir = tempfile::tempdir().unwrap();
    let path_a = dir.path().join("primary.log");
    let path_b = dir.path().join("secondary.log");
    let log = logger();

    let mut opts_a = HashMap::new();
    opts_a.insert("file_path".into(), path_a.to_string_lossy().into_owned());
    log.enable("file-a/", AuditBackend {
        path: "file-a/".into(),
        backend_type: AuditBackendType::File,
        description: "primary".into(),
        options: opts_a,
        local: false, seal_wrap: false,
    });

    let mut opts_b = HashMap::new();
    opts_b.insert("file_path".into(), path_b.to_string_lossy().into_owned());
    log.enable("file-b/", AuditBackend {
        path: "file-b/".into(),
        backend_type: AuditBackendType::File,
        description: "secondary".into(),
        options: opts_b,
        local: false, seal_wrap: false,
    });

    log.log(entry("hvs.X", "update", &format!("{}/secret/data/key", TENANT)));

    let a = std::fs::read_to_string(&path_a).expect("primary written");
    let b = std::fs::read_to_string(&path_b).expect("secondary written");
    assert_eq!(a.lines().count(), 1, "primary received one entry");
    assert_eq!(b.lines().count(), 1, "secondary received one entry");
    assert_eq!(a, b, "identical envelopes");

    // After disabling one backend, only the remaining one receives.
    assert!(log.disable("file-a/"));
    log.log(entry("hvs.X", "update", &format!("{}/secret/data/key2", TENANT)));
    let a_after = std::fs::read_to_string(&path_a).unwrap();
    let b_after = std::fs::read_to_string(&path_b).unwrap();
    assert_eq!(a_after.lines().count(), 1, "disabled backend stops receiving");
    assert_eq!(b_after.lines().count(), 2, "remaining backend keeps receiving");
}

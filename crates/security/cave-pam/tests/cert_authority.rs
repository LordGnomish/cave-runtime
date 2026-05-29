// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Tests for short-lived certificate issuance model.

use cave_pam::cert_authority::{
    CertAuthority, CertKind, CertRequest, IssuedCert,
};
use uuid::Uuid;
use chrono::Duration;

fn make_request(kind: CertKind, user: &str, ttl_hours: i64) -> CertRequest {
    CertRequest {
        requester_id: Uuid::new_v4(),
        principal: user.to_string(),
        kind,
        allowed_principals: vec![user.to_string(), "root".to_string()],
        ttl: Duration::hours(ttl_hours),
        extensions: std::collections::HashMap::new(),
    }
}

#[test]
fn test_issue_ssh_cert_returns_cert() {
    let ca = CertAuthority::new("test-cluster");
    let req = make_request(CertKind::Ssh, "alice", 8);
    let cert = ca.issue(req).expect("should issue SSH cert");
    assert_eq!(cert.kind, CertKind::Ssh);
    assert!(!cert.serial.is_empty());
    assert!(!cert.cert_pem.is_empty());
}

#[test]
fn test_issue_tls_cert_returns_cert() {
    let ca = CertAuthority::new("test-cluster");
    let req = make_request(CertKind::Tls, "bob", 4);
    let cert = ca.issue(req).expect("should issue TLS cert");
    assert_eq!(cert.kind, CertKind::Tls);
    assert!(!cert.cert_pem.is_empty());
}

#[test]
fn test_issued_cert_not_yet_expired() {
    let ca = CertAuthority::new("test-cluster");
    let cert = ca.issue(make_request(CertKind::Ssh, "carol", 1)).unwrap();
    assert!(!cert.is_expired());
}

#[test]
fn test_zero_ttl_cert_is_immediately_expired() {
    let ca = CertAuthority::new("test-cluster");
    let req = CertRequest {
        requester_id: Uuid::new_v4(),
        principal: "tmp".to_string(),
        kind: CertKind::Ssh,
        allowed_principals: vec!["tmp".to_string()],
        ttl: Duration::seconds(-1),
        extensions: std::collections::HashMap::new(),
    };
    let cert = ca.issue(req).unwrap();
    assert!(cert.is_expired());
}

#[test]
fn test_serials_are_unique() {
    let ca = CertAuthority::new("test-cluster");
    let c1 = ca.issue(make_request(CertKind::Ssh, "u1", 1)).unwrap();
    let c2 = ca.issue(make_request(CertKind::Ssh, "u2", 1)).unwrap();
    assert_ne!(c1.serial, c2.serial);
}

#[test]
fn test_cert_contains_principal() {
    let ca = CertAuthority::new("test-cluster");
    let cert = ca.issue(make_request(CertKind::Ssh, "dave", 1)).unwrap();
    assert_eq!(cert.principal, "dave");
}

#[test]
fn test_list_active_certs() {
    let ca = CertAuthority::new("test-cluster");
    let _c1 = ca.issue(make_request(CertKind::Ssh, "u1", 1)).unwrap();
    let _c2 = ca.issue(make_request(CertKind::Tls, "u2", 1)).unwrap();
    // One that is immediately expired
    let req_exp = CertRequest {
        requester_id: Uuid::new_v4(),
        principal: "expired_user".to_string(),
        kind: CertKind::Ssh,
        allowed_principals: vec!["expired_user".to_string()],
        ttl: Duration::seconds(-1),
        extensions: std::collections::HashMap::new(),
    };
    let _c3 = ca.issue(req_exp).unwrap();

    let active = ca.list_active();
    assert_eq!(active.len(), 2);
}

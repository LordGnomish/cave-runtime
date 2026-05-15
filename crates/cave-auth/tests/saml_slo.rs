// SPDX-License-Identifier: AGPL-3.0-or-later
//! RED phase for SAML 2.0 Single-Logout: <samlp:LogoutRequest>,
//! <samlp:LogoutResponse>, SOAP back-channel binding, and session-
//! index tracking.
//!
//! Source: keycloak/keycloak@b825ba97
//!         saml-core/src/main/java/org/keycloak/saml/processing/core/saml/v2/protocol/LogoutRequestType.java
//!         saml-core/src/main/java/org/keycloak/saml/SAML2LogoutResponseBuilder.java
//!         services/src/main/java/org/keycloak/protocol/saml/profile/util/LogoutProtocolUtil.java
//!         services/src/main/java/org/keycloak/broker/saml/SAMLEndpoint.java::logoutRequest

use cave_auth::saml::slo::{
    parse_logout_request, parse_logout_response, write_logout_request, write_logout_response,
    LogoutRequest, LogoutResponse, SessionIndexLedger, SLO_STATUS_PARTIAL_LOGOUT,
};
use cave_auth::saml::response::StatusCode;
use cave_auth::saml::NameIdFormat;

#[test]
fn logout_request_xml_round_trips() {
    let r = LogoutRequest {
        id: "_lr-1".into(),
        issue_instant: chrono::Utc::now(),
        destination: "https://idp.example/slo".into(),
        issuer: "https://sp.example".into(),
        name_id: "alice@example.com".into(),
        name_id_format: NameIdFormat::EmailAddress,
        session_indexes: vec!["sess-idx-1".into(), "sess-idx-2".into()],
        reason: Some("urn:oasis:names:tc:SAML:2.0:logout:user".into()),
    };
    let xml = write_logout_request(&r).unwrap();
    let parsed = parse_logout_request(&xml).unwrap();
    assert_eq!(parsed.id, r.id);
    assert_eq!(parsed.issuer, r.issuer);
    assert_eq!(parsed.name_id, "alice@example.com");
    assert_eq!(parsed.name_id_format, NameIdFormat::EmailAddress);
    assert_eq!(parsed.session_indexes.len(), 2);
    assert_eq!(parsed.session_indexes[0], "sess-idx-1");
}

#[test]
fn logout_response_xml_round_trips() {
    let r = LogoutResponse {
        id: "_lresp-1".into(),
        issue_instant: chrono::Utc::now(),
        destination: "https://sp.example/slo-ack".into(),
        in_response_to: "_lr-1".into(),
        issuer: "https://idp.example".into(),
        status: StatusCode::Success,
    };
    let xml = write_logout_response(&r).unwrap();
    let parsed = parse_logout_response(&xml).unwrap();
    assert_eq!(parsed.in_response_to, "_lr-1");
    assert_eq!(parsed.status, StatusCode::Success);
    assert_eq!(parsed.issuer, "https://idp.example");
}

#[test]
fn session_index_ledger_drops_indexes_on_logout() {
    let ledger = SessionIndexLedger::new();
    ledger.track("alice@example.com", "sess-1");
    ledger.track("alice@example.com", "sess-2");
    ledger.track("bob@example.com", "sess-99");
    assert_eq!(ledger.indexes_for("alice@example.com").len(), 2);

    // Logout one of alice's sessions.
    ledger.drop_index("alice@example.com", "sess-1");
    assert_eq!(ledger.indexes_for("alice@example.com"), vec!["sess-2"]);
    // bob unaffected.
    assert_eq!(ledger.indexes_for("bob@example.com"), vec!["sess-99"]);
}

#[test]
fn session_index_ledger_logout_all_for_principal() {
    let ledger = SessionIndexLedger::new();
    ledger.track("alice", "s1");
    ledger.track("alice", "s2");
    let dropped = ledger.drop_all("alice");
    assert_eq!(dropped.len(), 2);
    assert!(ledger.indexes_for("alice").is_empty());
}

#[test]
fn partial_logout_status_urn_is_spec_value() {
    assert_eq!(
        SLO_STATUS_PARTIAL_LOGOUT,
        "urn:oasis:names:tc:SAML:2.0:status:PartialLogout"
    );
}

#[test]
fn parse_logout_request_rejects_malformed_xml() {
    assert!(parse_logout_request(b"<not xml").is_err());
}

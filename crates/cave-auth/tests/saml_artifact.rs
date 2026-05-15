// SPDX-License-Identifier: AGPL-3.0-or-later
//! RED phase for SAML 2.0 HTTP-Artifact binding (§3.6) port.
//!
//! Source: keycloak/keycloak@b825ba97
//!         saml-core/src/main/java/org/keycloak/saml/processing/core/saml/v2/protocol/ArtifactResponse{Type}.java
//!         saml-core/src/main/java/org/keycloak/saml/processing/core/saml/v2/protocol/ArtifactResolveType.java
//!         saml-core-api/src/main/java/org/keycloak/saml/common/constants/JBossSAMLURIConstants.java
//!         services/.../broker/saml/SAMLEndpoint.java::handleArtifactResponse
//!
//! HTTP-Artifact binding flow: SP receives a short `SAMLart=` artifact ID
//! over the front channel and resolves it back-channel by POSTing a SOAP
//! `<ArtifactResolve>` to the IdP's `ArtifactResolutionService`. The IdP
//! replies with a SOAP `<ArtifactResponse>` wrapping the actual
//! `<samlp:Response>`. This test set drives the parser, writer, and
//! resolver state machine; impl lands in src/saml/artifact.rs.
//!
//! These tests intentionally reference items that do not exist yet
//! (`cave_auth::saml::artifact`) — they are the RED proof. The GREEN
//! commit lands the smallest implementation that makes them pass.

#![allow(clippy::needless_doctest_main)]

use cave_auth::saml::artifact::{
    parse_artifact_resolve, parse_artifact_response, write_artifact_resolve,
    write_artifact_response, Artifact, ArtifactResolutionStore, ArtifactResolve,
    ArtifactResponse, ARTIFACT_BINDING_URN,
};
use cave_auth::saml::response::{Assertion, Response, StatusCode};

#[test]
fn artifact_binding_urn_is_spec_value() {
    assert_eq!(
        ARTIFACT_BINDING_URN,
        "urn:oasis:names:tc:SAML:2.0:bindings:HTTP-Artifact"
    );
}

#[test]
fn artifact_type_code_is_0x0004_per_spec() {
    // §3.6.4 SAML 2.0 Bindings: TypeCode = 0x0004 for the type-4 artifact
    // every real IdP emits.
    let a = Artifact::new_type4(b"my-source-id-20-bytes", b"01234567890123456789");
    let bytes = a.to_bytes();
    assert_eq!(&bytes[0..2], &[0x00, 0x04], "TypeCode bytes 0,1 = 0x0004");
}

#[test]
fn artifact_round_trips_through_base64() {
    let a = Artifact::new_type4(b"sourceidentifie20by!", b"messagehandle20bytes");
    let s = a.to_base64();
    let parsed = Artifact::from_base64(&s).unwrap();
    assert_eq!(parsed.source_id, a.source_id);
    assert_eq!(parsed.message_handle, a.message_handle);
    assert_eq!(parsed.endpoint_index, a.endpoint_index);
}

#[test]
fn artifact_rejects_short_payload() {
    // Real artifact is 44 bytes — anything shorter is malformed.
    let short = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        b"too short",
    );
    assert!(Artifact::from_base64(&short).is_err());
}

#[test]
fn artifact_resolve_xml_round_trips() {
    let r = ArtifactResolve {
        id: "_resolve-1".into(),
        issue_instant: chrono::Utc::now(),
        issuer: "https://sp.example".into(),
        artifact: "AAQAAA...".into(),
        destination: "https://idp.example/ars".into(),
    };
    let xml = write_artifact_resolve(&r).unwrap();
    let parsed = parse_artifact_resolve(&xml).unwrap();
    assert_eq!(parsed.id, r.id);
    assert_eq!(parsed.issuer, r.issuer);
    assert_eq!(parsed.artifact, r.artifact);
    assert_eq!(parsed.destination, r.destination);
    // SOAP envelope is preserved on the wire.
    let s = std::str::from_utf8(&xml).unwrap();
    assert!(s.contains("Envelope"), "wraps in SOAP Envelope");
    assert!(
        s.contains("ArtifactResolve") || s.contains("samlp:ArtifactResolve"),
        "carries samlp:ArtifactResolve"
    );
}

#[test]
fn artifact_response_wraps_inner_saml_response() {
    let inner = Response::success(
        "https://idp.example",
        "https://sp.example/acs",
        Some("_req-1".into()),
        Assertion::new("https://idp.example", "alice@example.com"),
    );
    let outer = ArtifactResponse {
        id: "_aresp-1".into(),
        issue_instant: chrono::Utc::now(),
        in_response_to: "_resolve-1".into(),
        issuer: "https://idp.example".into(),
        status: StatusCode::Success,
        inner_response: Some(inner.clone()),
    };
    let xml = write_artifact_response(&outer).unwrap();
    let parsed = parse_artifact_response(&xml).unwrap();
    assert_eq!(parsed.id, outer.id);
    assert_eq!(parsed.in_response_to, "_resolve-1");
    assert_eq!(parsed.status, StatusCode::Success);
    let i = parsed.inner_response.expect("inner Response decoded");
    assert_eq!(i.in_response_to.as_deref(), Some("_req-1"));
    assert_eq!(i.issuer, inner.issuer);
}

#[test]
fn artifact_resolution_store_resolves_once_then_evicts() {
    let store = ArtifactResolutionStore::new();
    let a = Artifact::new_type4(b"sourceidentifie20by!", b"messagehandle20bytes");
    let r = Response::success(
        "https://idp.example",
        "https://sp.example/acs",
        Some("_req-1".into()),
        Assertion::new("https://idp.example", "u"),
    );
    store.put(a.to_base64(), r.clone());
    // First resolve hits — the IdP gets exactly one shot per artifact.
    let got = store.take(&a.to_base64()).unwrap();
    assert_eq!(got.id, r.id);
    // Second resolve must miss — the store is single-shot per §3.6.5.
    assert!(store.take(&a.to_base64()).is_none());
}

#[test]
fn artifact_resolution_store_misses_unknown_artifact() {
    let store = ArtifactResolutionStore::new();
    assert!(store.take("AAQA-unknown").is_none());
}

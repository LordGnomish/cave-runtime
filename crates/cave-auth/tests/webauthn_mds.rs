// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: webauthn4j/webauthn4j@82345b8
//   webauthn4j-metadata/src/test/java/com/webauthn4j/metadata/MetadataBLOBPayloadTest.java
//
// RED — FIDO Alliance Metadata Service v3 (MDS3) blob structural parse.
// We parse the JWT in three parts (header.payload.sig) and decode the
// `entries[]` array.  Full chain validation against the FIDO root CA
// is an honest scope-cut.

use cave_auth::webauthn::mds::{MdsBlob, MdsError, MetadataStatement};

fn build_unsigned_jwt(payload: &serde_json::Value) -> String {
    use base64::Engine;
    let header = serde_json::json!({"alg": "ES256", "typ": "JWT", "x5c": []});
    let h = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(header.to_string());
    let p = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload.to_string());
    // We don't verify the signature in tests — pass a placeholder.
    let s = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b"fake-sig");
    format!("{h}.{p}.{s}")
}

#[test]
fn parses_minimal_mds_blob() {
    let payload = serde_json::json!({
        "legalHeader": "https://fidoalliance.org/metadata/legal/",
        "no": 42,
        "nextUpdate": "2026-06-01",
        "entries": [
            {
                "aaguid": "00000000-0000-0000-0000-000000000000",
                "statusReports": [
                    {"status": "FIDO_CERTIFIED_L1", "effectiveDate": "2024-01-01"}
                ],
                "timeOfLastStatusChange": "2024-01-01",
                "metadataStatement": {
                    "description": "Test Authenticator",
                    "authenticatorVersion": 1,
                    "protocolFamily": "fido2",
                    "schema": 3,
                    "aaguid": "00000000-0000-0000-0000-000000000000"
                }
            }
        ]
    });
    let jwt = build_unsigned_jwt(&payload);
    let blob = MdsBlob::parse_unsigned(&jwt).unwrap();
    assert_eq!(blob.no, 42);
    assert_eq!(blob.entries.len(), 1);
    let entry = &blob.entries[0];
    assert_eq!(
        entry.aaguid.as_deref(),
        Some("00000000-0000-0000-0000-000000000000")
    );
    assert_eq!(entry.metadata_statement.as_ref().map(|m| m.protocol_family.as_str()), Some("fido2"));
}

#[test]
fn lookup_by_aaguid_finds_entry() {
    let payload = serde_json::json!({
        "legalHeader": "x",
        "no": 1,
        "nextUpdate": "2030-01-01",
        "entries": [
            { "aaguid": "fa2b99dc-9e39-4257-8f92-4a30d23c4118",
              "statusReports": [],
              "timeOfLastStatusChange": "2024-01-01",
              "metadataStatement": {
                  "description": "YubiKey 5 Series",
                  "authenticatorVersion": 50100,
                  "protocolFamily": "fido2",
                  "schema": 3,
                  "aaguid": "fa2b99dc-9e39-4257-8f92-4a30d23c4118"
              }
            }
        ]
    });
    let jwt = build_unsigned_jwt(&payload);
    let blob = MdsBlob::parse_unsigned(&jwt).unwrap();
    let entry = blob.find_by_aaguid("fa2b99dc-9e39-4257-8f92-4a30d23c4118").unwrap();
    assert_eq!(
        entry.metadata_statement.as_ref().unwrap().description,
        "YubiKey 5 Series"
    );
}

#[test]
fn rejects_malformed_jwt() {
    let err = MdsBlob::parse_unsigned("not.a.jwt.too.many.dots").unwrap_err();
    assert!(matches!(err, MdsError::BadJwt));

    let err = MdsBlob::parse_unsigned("only.one").unwrap_err();
    assert!(matches!(err, MdsError::BadJwt));
}

#[test]
fn rejects_garbled_payload() {
    let bad = "aGVhZGVy.notvalidbase64@@@.c2ln";
    assert!(matches!(MdsBlob::parse_unsigned(bad), Err(_)));
}

#[test]
fn metadata_statement_carries_authenticator_version() {
    let ms_json = serde_json::json!({
        "description": "Foo",
        "authenticatorVersion": 12345,
        "protocolFamily": "fido2",
        "schema": 3,
        "aaguid": "00000000-0000-0000-0000-000000000000"
    });
    let ms: MetadataStatement = serde_json::from_value(ms_json).unwrap();
    assert_eq!(ms.authenticator_version, 12345);
    assert_eq!(ms.description, "Foo");
}

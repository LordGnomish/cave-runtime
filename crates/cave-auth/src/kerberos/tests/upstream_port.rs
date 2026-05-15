// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 federation/kerberos/src/test/.../

//! Upstream-test port. Each `#[test]` mirrors a Keycloak
//! `KerberosFederationProviderTest` /
//! `SPNEGOAuthenticatorTest` / `KeyTabReaderTest` fixture.

use crate::kerberos::{
    gssapi::{wrap_initial_context_token, InitialContextToken, OID_KRB5, OID_SPNEGO},
    keytab::{encode_test_keytab, parse_keytab, KeyBlock, KeytabEntry, KrbPrincipal, KEYTAB_MAGIC},
    negotiate::NegotiateHandler,
    spnego::{build_neg_token_init, NegState, NegTokenInit, NegTokenResp},
};

fn aes256_entry() -> KeytabEntry {
    KeytabEntry {
        principal: KrbPrincipal {
            realm: "EXAMPLE.COM".into(),
            components: vec!["HTTP".into(), "cave.example.com".into()],
            name_type: 1,
        },
        timestamp: 1_700_000_000,
        vno: 7,
        key: KeyBlock {
            enctype: 18,
            contents: vec![0u8; 32],
        },
    }
}

#[test]
fn spnego_authenticator_test_init_token_round_trips() {
    let oids: &[&[u8]] = &[OID_KRB5];
    let token = [0x01, 0x02, 0x03];
    let init = build_neg_token_init(oids, Some(&token));
    let wrapped = wrap_initial_context_token(OID_SPNEGO, &init);
    let gss = InitialContextToken::parse(&wrapped).unwrap();
    assert!(gss.is_spnego());
    let inner = NegTokenInit::parse(gss.inner).unwrap();
    assert_eq!(inner.mech_types, vec![OID_KRB5]);
    assert_eq!(inner.mech_token, Some(&token[..]));
}

#[test]
fn spnego_authenticator_test_neg_state_accept_completed() {
    let bytes = [0xa1, 0x07, 0x30, 0x05, 0xa0, 0x03, 0x0a, 0x01, 0x00];
    let resp = NegTokenResp::parse(&bytes).unwrap();
    assert_eq!(resp.neg_state, Some(NegState::AcceptCompleted));
}

#[test]
fn kerberos_federation_provider_test_challenge_header_value() {
    let h = NegotiateHandler::new();
    let (status, headers) = h.unauthorized_response();
    assert_eq!(status, 401);
    assert_eq!(headers[0].1, "Negotiate");
}

#[test]
fn kerberos_federation_provider_test_authorization_header_decode() {
    use base64::Engine;
    let h = NegotiateHandler::new();
    let token = [0xaa];
    let inner = build_neg_token_init(&[OID_KRB5], Some(&token));
    let wrapped = wrap_initial_context_token(OID_SPNEGO, &inner);
    let header = format!(
        "Negotiate {}",
        base64::engine::general_purpose::STANDARD.encode(&wrapped)
    );
    let decoded = h.decode_request(&header).unwrap();
    assert!(decoded.has_known_mech());
    assert_eq!(decoded.mech_token, Some(token.to_vec()));
}

#[test]
fn key_tab_reader_test_magic_number_is_0x0502() {
    let bytes = encode_test_keytab(&[aes256_entry()]);
    assert_eq!(&bytes[..2], &KEYTAB_MAGIC.to_be_bytes());
}

#[test]
fn key_tab_reader_test_parses_aes256_entry() {
    let entry = aes256_entry();
    let bytes = encode_test_keytab(&[entry.clone()]);
    let parsed = parse_keytab(&bytes).unwrap();
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].key.enctype, 18);
}

#[test]
fn key_tab_reader_test_principal_canonical_form() {
    let entry = aes256_entry();
    assert_eq!(
        entry.principal.to_canonical(),
        "HTTP/cave.example.com@EXAMPLE.COM"
    );
}

#[test]
fn key_tab_reader_test_rejects_v1_format() {
    let err = parse_keytab(&[0x05, 0x01, 0x00, 0x00]).unwrap_err();
    assert!(matches!(err, crate::kerberos::KerberosError::Keytab(_)));
}

#[test]
fn key_tab_reader_test_two_entries_preserved() {
    let a = aes256_entry();
    let mut b = a.clone();
    b.principal.components = vec!["host".into(), "node.example.com".into()];
    b.key.enctype = 17;
    b.key.contents = vec![0u8; 16];
    let bytes = encode_test_keytab(&[a.clone(), b.clone()]);
    let parsed = parse_keytab(&bytes).unwrap();
    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed[0].principal.components[0], "HTTP");
    assert_eq!(parsed[1].principal.components[0], "host");
}

//! cave-acme — Account model tests pinned to RFC 8555.

use cave_acme::{Account, AccountStatus, ExternalAccountBinding, Jwk};

const TENANT: &str = "tenant-acme-prod";

fn ed25519_jwk() -> Jwk {
    Jwk::OKP { crv: "Ed25519".into(),
        x: "11qYAYKxCrfVS_7TyWQHOg7hcvPapiMlrwIaaPcHURo".into() }
}

/// Cite: RFC 7638 §3 — JWK thumbprint MUST be deterministic and based
/// on the canonical JSON of the required members in alphabetical order.
#[test]
fn jwk_thumbprint_is_deterministic() {
    let jwk = ed25519_jwk();
    let t1 = jwk.thumbprint();
    let t2 = jwk.thumbprint();
    assert_eq!(t1, t2, "thumbprint must be deterministic");
    assert!(!t1.is_empty(), "thumbprint must be non-empty");

    // Different JWK ⇒ different thumbprint.
    let other = Jwk::OKP { crv: "Ed25519".into(),
        x: "different-base64-value".into() };
    assert_ne!(jwk.thumbprint(), other.thumbprint());
}

/// Cite: RFC 8555 §8.1 — `keyAuthorization = token + "." + thumbprint`.
#[test]
fn key_authorization_format() {
    let jwk = ed25519_jwk();
    let token = "AAAA-token-1234";
    let auth = jwk.key_authorization(token);
    assert!(auth.starts_with(token));
    assert!(auth.contains('.'));
    let parts: Vec<&str> = auth.splitn(2, '.').collect();
    assert_eq!(parts[0], token);
    assert_eq!(parts[1], jwk.thumbprint());
}

/// Cite: RFC 8555 §7.3 (newAccount validation): contact URLs must use
/// the `mailto:` scheme; ToS agreement is mandatory.
#[test]
fn account_validation_rejects_bad_contact_and_missing_tos() {
    let jwk = ed25519_jwk();
    let now = chrono::Utc::now();
    let mut a = Account {
        id: "acct-1".into(),
        tenant_id: TENANT.into(),
        status: AccountStatus::Valid,
        contact: vec!["https://example.com/contact".into()],
        terms_of_service_agreed: true,
        jwk: jwk.clone(),
        eab: None,
        created_at: now,
    };
    let err = a.validate().unwrap_err();
    assert!(err.to_string().contains("mailto"));

    a.contact = vec![format!("mailto:ops@{}.cave-runtime.test", TENANT)];
    a.terms_of_service_agreed = false;
    let err = a.validate().unwrap_err();
    assert!(err.to_string().contains("terms of service"));

    a.terms_of_service_agreed = true;
    assert!(a.validate().is_ok());
}

/// Cite: RFC 8555 §7.3.4 — EAB MUST use a supported MAC algorithm
/// (HS256 in cave today); kid + mac MUST be non-empty.
#[test]
fn external_account_binding_validates_algorithm_and_required_fields() {
    let jwk = ed25519_jwk();
    let mut a = Account {
        id: "acct-eab".into(),
        tenant_id: TENANT.into(),
        status: AccountStatus::Valid,
        contact: vec![format!("mailto:ops@{}.cave-runtime.test", TENANT)],
        terms_of_service_agreed: true,
        jwk,
        eab: Some(ExternalAccountBinding {
            kid: "operator-key-1".into(),
            alg: "HS512".into(),  // unsupported
            mac: "BASE64URL-MAC".into(),
        }),
        created_at: chrono::Utc::now(),
    };
    let err = a.validate().unwrap_err();
    assert!(err.to_string().contains("unsupported EAB alg"));

    a.eab = Some(ExternalAccountBinding {
        kid: "".into(),  // missing
        alg: "HS256".into(),
        mac: "BASE64URL-MAC".into(),
    });
    assert!(a.validate().is_err());

    a.eab = Some(ExternalAccountBinding {
        kid: "operator-key-1".into(),
        alg: "HS256".into(),
        mac: "BASE64URL-MAC".into(),
    });
    assert!(a.validate().is_ok());
}

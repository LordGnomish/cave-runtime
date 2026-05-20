// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: webauthn4j@v0.24.0 + keycloak@v22.0.0 services/.../webauthn + W3C WebAuthn L2
//
// clientDataJSON parsing — webauthn4j `data.client.CollectedClientData`.

use serde::Deserialize;

use super::WebAuthnError;

/// W3C WebAuthn L2 §5.8.1 — CollectedClientData.
///
/// Fields are kept verbatim from the JSON wire form (camelCase) for direct
/// re-encoding. webauthn4j stores this as a Java POJO with the same names.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct CollectedClientData {
    /// `webauthn.create` for registration, `webauthn.get` for assertion.
    #[serde(rename = "type")]
    pub typ: String,
    /// Base64URL-encoded challenge (no padding) — must match the server-
    /// issued challenge byte-for-byte after base64url-decoding.
    pub challenge: String,
    /// RP origin — `https://login.example.com`.
    pub origin: String,
    /// True if a top-level cross-origin iframe initiated the request
    /// (per L2 §5.8.1.2). Servers usually reject `true`.
    #[serde(rename = "crossOrigin", default)]
    pub cross_origin: bool,
    /// Optional `tokenBinding` block — rarely used in practice.
    #[serde(rename = "tokenBinding", default)]
    pub token_binding: Option<TokenBinding>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct TokenBinding {
    pub status: String,
    #[serde(default)]
    pub id: Option<String>,
}

/// Operation type. Used as an enum guard so callers can't mistake create vs get.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientDataType {
    Create,
    Get,
}

impl ClientDataType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Create => "webauthn.create",
            Self::Get => "webauthn.get",
        }
    }
}

/// Parse a clientDataJSON byte stream.
///
/// Port of webauthn4j `CollectedClientDataConverter#convert`.
pub fn parse(raw: &[u8]) -> Result<CollectedClientData, WebAuthnError> {
    serde_json::from_slice(raw).map_err(|e| WebAuthnError::ClientData(format!("json: {e}")))
}

/// Verify the four invariants required by W3C §7.1 steps 11-14 (registration)
/// and §7.2 steps 12-15 (authentication). Returns Ok(()) only if all pass.
pub fn verify(
    cd: &CollectedClientData,
    expected_type: ClientDataType,
    expected_challenge: &[u8],
    expected_origin: &str,
) -> Result<(), WebAuthnError> {
    // Step 11/12 — type.
    if cd.typ != expected_type.as_str() {
        return Err(WebAuthnError::ClientData(format!(
            "type mismatch: got {:?}, want {:?}",
            cd.typ,
            expected_type.as_str()
        )));
    }
    // Step 12/13 — challenge.
    use base64::Engine as _;
    let got =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(cd.challenge.trim_end_matches('='));
    let got = got.map_err(|e| WebAuthnError::ClientData(format!("challenge b64: {e}")))?;
    if got != expected_challenge {
        return Err(WebAuthnError::ClientData("challenge mismatch".into()));
    }
    // Step 13/14 — origin.
    if cd.origin != expected_origin {
        return Err(WebAuthnError::ClientData(format!(
            "origin mismatch: got {:?}, want {:?}",
            cd.origin, expected_origin
        )));
    }
    // Step 14/15 — cross-origin guard.
    if cd.cross_origin {
        return Err(WebAuthnError::ClientData("cross-origin not allowed".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;

    fn b64u(s: &[u8]) -> String {
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(s)
    }

    fn fixture(typ: &str, ch: &[u8], origin: &str) -> Vec<u8> {
        format!(
            r#"{{"type":"{typ}","challenge":"{ch}","origin":"{origin}","crossOrigin":false}}"#,
            typ = typ,
            ch = b64u(ch),
            origin = origin
        )
        .into_bytes()
    }

    #[test]
    fn parse_registration_clientdata() {
        let raw = fixture("webauthn.create", b"hello", "https://login.cave.dev");
        let cd = parse(&raw).unwrap();
        assert_eq!(cd.typ, "webauthn.create");
        assert_eq!(cd.origin, "https://login.cave.dev");
    }

    #[test]
    fn verify_happy_path() {
        let raw = fixture("webauthn.create", b"abc", "https://login.cave.dev");
        let cd = parse(&raw).unwrap();
        verify(
            &cd,
            ClientDataType::Create,
            b"abc",
            "https://login.cave.dev",
        )
        .unwrap();
    }

    #[test]
    fn verify_type_mismatch() {
        let raw = fixture("webauthn.get", b"abc", "https://login.cave.dev");
        let cd = parse(&raw).unwrap();
        assert!(
            verify(
                &cd,
                ClientDataType::Create,
                b"abc",
                "https://login.cave.dev"
            )
            .is_err()
        );
    }

    #[test]
    fn verify_challenge_mismatch() {
        let raw = fixture("webauthn.create", b"abc", "https://login.cave.dev");
        let cd = parse(&raw).unwrap();
        assert!(
            verify(
                &cd,
                ClientDataType::Create,
                b"xyz",
                "https://login.cave.dev"
            )
            .is_err()
        );
    }

    #[test]
    fn verify_origin_mismatch() {
        let raw = fixture("webauthn.create", b"abc", "https://evil.example");
        let cd = parse(&raw).unwrap();
        assert!(
            verify(
                &cd,
                ClientDataType::Create,
                b"abc",
                "https://login.cave.dev"
            )
            .is_err()
        );
    }

    #[test]
    fn verify_cross_origin_rejected() {
        let raw = br#"{"type":"webauthn.create","challenge":"YWJj","origin":"https://login.cave.dev","crossOrigin":true}"#;
        let cd = parse(raw).unwrap();
        assert!(
            verify(
                &cd,
                ClientDataType::Create,
                b"abc",
                "https://login.cave.dev"
            )
            .is_err()
        );
    }

    #[test]
    fn malformed_json_returns_error() {
        assert!(parse(b"{not-json").is_err());
    }
}

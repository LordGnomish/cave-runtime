// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@b825ba97 federation/ldap/src/main/java/org/keycloak/storage/ldap/idm/store/ldap/LDAPIdentityStore.java
// Source: keycloak/keycloak@b825ba97 federation/ldap/src/main/java/org/keycloak/storage/ldap/idm/store/ldap/LDAPContextManager.java
//
// LDAPv3 BindRequest / BindResponse, RFC 4511 §4.2.
//
//    BindRequest ::= [APPLICATION 0] SEQUENCE {
//         version                 INTEGER (1 ..  127),
//         name                    LDAPDN,
//         authentication          AuthenticationChoice }
//
//    AuthenticationChoice ::= CHOICE {
//         simple                  [0] OCTET STRING,
//         sasl                    [3] SaslCredentials,
//         ... }
//
//    SaslCredentials ::= SEQUENCE {
//         mechanism               LDAPString,
//         credentials             OCTET STRING OPTIONAL }
//
// We support the four authentication methods Keycloak speaks:
//   * Simple bind (DN + password)
//   * SASL PLAIN     (RFC 4616)
//   * SASL EXTERNAL  (RFC 4422 §7.4 — used for mTLS)
//   * SASL GSSAPI    (RFC 4752 — request frame only; ticket
//                     verification happens in `federation::kerberos`)

use super::ber::{
    self, integer, octet_string, sequence, Decoder, Element, Form, Tag,
};

/// LDAP result-code values relevant to bind/search.  Verbatim
/// from RFC 4511 §4.1.9 + drafts; identical to
/// `LDAPException` in JNDI.
pub mod result {
    pub const SUCCESS: u32 = 0;
    pub const OPERATIONS_ERROR: u32 = 1;
    pub const PROTOCOL_ERROR: u32 = 2;
    pub const TIME_LIMIT_EXCEEDED: u32 = 3;
    pub const SIZE_LIMIT_EXCEEDED: u32 = 4;
    pub const NO_SUCH_OBJECT: u32 = 32;
    pub const INVALID_DN_SYNTAX: u32 = 34;
    pub const INVALID_CREDENTIALS: u32 = 49;
    pub const INSUFFICIENT_ACCESS_RIGHTS: u32 = 50;
    pub const BUSY: u32 = 51;
    pub const UNAVAILABLE: u32 = 52;
    pub const UNWILLING_TO_PERFORM: u32 = 53;
    pub const SASL_BIND_IN_PROGRESS: u32 = 14;
    pub const REFERRAL: u32 = 10;
}

/// Authentication choice (RFC 4511 §4.2.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthChoice {
    Simple(Vec<u8>),
    SaslPlain { authzid: String, authcid: String, password: String },
    /// SASL EXTERNAL — credentials are out-of-band (e.g. TLS client
    /// cert).  May carry an authzid.
    SaslExternal { authzid: Option<String> },
    /// SASL GSSAPI initial response — Keycloak builds this from the
    /// raw GSS-API output token; we accept a pre-built token here.
    SaslGssapi { token: Vec<u8> },
}

/// LDAP BindRequest envelope.  `message_id` is the outer LDAPMessage
/// component shared by all operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindRequest {
    pub message_id: u32,
    pub version: u8,
    pub name: String,
    pub auth: AuthChoice,
}

impl BindRequest {
    pub fn simple(message_id: u32, dn: impl Into<String>, password: impl Into<Vec<u8>>) -> Self {
        Self {
            message_id,
            version: 3,
            name: dn.into(),
            auth: AuthChoice::Simple(password.into()),
        }
    }

    pub fn sasl_plain(message_id: u32, authcid: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            message_id,
            version: 3,
            name: String::new(),
            auth: AuthChoice::SaslPlain {
                authzid: String::new(),
                authcid: authcid.into(),
                password: password.into(),
            },
        }
    }

    pub fn sasl_external(message_id: u32, authzid: Option<String>) -> Self {
        Self {
            message_id,
            version: 3,
            name: String::new(),
            auth: AuthChoice::SaslExternal { authzid },
        }
    }

    pub fn sasl_gssapi(message_id: u32, token: Vec<u8>) -> Self {
        Self {
            message_id,
            version: 3,
            name: String::new(),
            auth: AuthChoice::SaslGssapi { token },
        }
    }

    /// Serialize this BindRequest as a complete LDAPMessage frame.
    pub fn encode(&self) -> Vec<u8> {
        // BindRequest is [APPLICATION 0] (constructed).
        let auth_elem: Element = match &self.auth {
            AuthChoice::Simple(pw) => {
                // [0] OCTET STRING, primitive.
                Element::new(Tag::context(0, Form::Primitive), pw.clone())
            }
            AuthChoice::SaslPlain { authzid, authcid, password } => {
                // RFC 4616: authzid \0 authcid \0 password
                let mut creds = Vec::new();
                creds.extend_from_slice(authzid.as_bytes());
                creds.push(0);
                creds.extend_from_slice(authcid.as_bytes());
                creds.push(0);
                creds.extend_from_slice(password.as_bytes());
                let inner = sequence(&[
                    octet_string(b"PLAIN"),
                    octet_string(&creds),
                ]);
                // [3] SaslCredentials, constructed.
                Element::new(Tag::context(3, Form::Constructed), inner.bytes)
            }
            AuthChoice::SaslExternal { authzid } => {
                let mut children = vec![octet_string(b"EXTERNAL")];
                if let Some(z) = authzid {
                    children.push(octet_string(z.as_bytes()));
                }
                let inner = sequence(&children);
                Element::new(Tag::context(3, Form::Constructed), inner.bytes)
            }
            AuthChoice::SaslGssapi { token } => {
                let inner = sequence(&[octet_string(b"GSSAPI"), octet_string(token)]);
                Element::new(Tag::context(3, Form::Constructed), inner.bytes)
            }
        };

        let body = vec![
            integer(self.version as i64),
            octet_string(self.name.as_bytes()),
            auth_elem,
        ];
        let mut body_bytes = Vec::new();
        for e in &body {
            body_bytes.extend_from_slice(&e.encode());
        }
        let bind_request = Element::new(Tag::application(0, Form::Constructed), body_bytes);

        let envelope = sequence(&[integer(self.message_id as i64), bind_request]);
        envelope.encode()
    }
}

/// LDAP BindResponse — RFC 4511 §4.2.2.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindResponse {
    pub message_id: u32,
    pub result_code: u32,
    pub matched_dn: String,
    pub diagnostic_message: String,
    /// SASL serverSaslCreds (optional, context-tag [7]).
    pub server_sasl_creds: Option<Vec<u8>>,
}

impl BindResponse {
    /// Parse an LDAPMessage that wraps a BindResponse ([APPLICATION 1]).
    pub fn decode(frame: &[u8]) -> Result<Self, ber::DecodeError> {
        let mut d = Decoder::new(frame);
        let envelope = d.read_expected(Tag::universal(16, Form::Constructed))?;
        let mut e = Decoder::new(envelope);
        let message_id = e.read_integer()? as u32;

        let body = e.read_expected(Tag::application(1, Form::Constructed))?;
        let mut b = Decoder::new(body);
        let result_code = b.read_enumerated()? as u32;
        let matched_dn = b.read_octet_string_utf8()?;
        let diagnostic_message = b.read_octet_string_utf8()?;
        let mut server_sasl_creds = None;
        if !b.eof() {
            let (tag, payload) = b.read_tlv()?;
            if tag == Tag::context(7, Form::Primitive) {
                server_sasl_creds = Some(payload.to_vec());
            }
        }
        Ok(Self {
            message_id,
            result_code,
            matched_dn,
            diagnostic_message,
            server_sasl_creds,
        })
    }

    /// Helper for fixtures + tests — encode the response side, mirror
    /// of `decode` above.
    pub fn encode(&self) -> Vec<u8> {
        let body = sequence(&[
            ber::enumerated(self.result_code as i64),
            octet_string(self.matched_dn.as_bytes()),
            octet_string(self.diagnostic_message.as_bytes()),
        ]);
        let body = Element::new(Tag::application(1, Form::Constructed), body.bytes);
        let envelope = sequence(&[integer(self.message_id as i64), body]);
        envelope.encode()
    }
}

/// High-level summary used by the portal "Test bind" button.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindOutcome {
    Success,
    InvalidCredentials,
    SaslContinue(Vec<u8>),
    Other { code: u32, message: String },
}

impl BindOutcome {
    pub fn from_response(r: &BindResponse) -> Self {
        match r.result_code {
            result::SUCCESS => BindOutcome::Success,
            result::INVALID_CREDENTIALS => BindOutcome::InvalidCredentials,
            result::SASL_BIND_IN_PROGRESS => {
                BindOutcome::SaslContinue(r.server_sasl_creds.clone().unwrap_or_default())
            }
            code => BindOutcome::Other { code, message: r.diagnostic_message.clone() },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_bind_encodes_application_zero_constructed() {
        let req = BindRequest::simple(1, "cn=admin,dc=acme", "secret");
        let bytes = req.encode();
        // outer is universal SEQUENCE.
        assert_eq!(bytes[0], 0x30);
        // inside the envelope we should find an [APPLICATION 0]
        // constructed tag — 0x60.
        assert!(bytes.windows(1).any(|w| w[0] == 0x60));
    }

    #[test]
    fn simple_bind_carries_dn_and_password() {
        let req = BindRequest::simple(7, "cn=alice,dc=acme", "pw");
        let bytes = req.encode();
        // Both DN bytes and the password bytes appear in the frame.
        assert!(bytes.windows(b"cn=alice,dc=acme".len()).any(|w| w == b"cn=alice,dc=acme"));
        assert!(bytes.windows(b"pw".len()).any(|w| w == b"pw"));
    }

    #[test]
    fn sasl_plain_uses_context_tag_three_constructed() {
        let req = BindRequest::sasl_plain(1, "alice", "pw");
        let bytes = req.encode();
        // [3] SaslCredentials → class=10, form=1 → 0xa3.
        assert!(bytes.contains(&0xa3));
        // Mechanism name appears verbatim.
        assert!(bytes.windows(5).any(|w| w == b"PLAIN"));
    }

    #[test]
    fn sasl_external_optional_authzid_omitted_by_default() {
        let req = BindRequest::sasl_external(1, None);
        let bytes = req.encode();
        assert!(bytes.windows(8).any(|w| w == b"EXTERNAL"));
        let with_z = BindRequest::sasl_external(1, Some("u:alice".into())).encode();
        assert!(with_z.windows(7).any(|w| w == b"u:alice"));
    }

    #[test]
    fn sasl_gssapi_carries_token_bytes() {
        let token = vec![0x60, 0x82, 0x01, 0x02, 0xde, 0xad];
        let req = BindRequest::sasl_gssapi(1, token.clone());
        let bytes = req.encode();
        assert!(bytes.windows(token.len()).any(|w| w == token.as_slice()));
        assert!(bytes.windows(6).any(|w| w == b"GSSAPI"));
    }

    #[test]
    fn bind_response_decodes_success() {
        let r = BindResponse {
            message_id: 1,
            result_code: 0,
            matched_dn: String::new(),
            diagnostic_message: String::new(),
            server_sasl_creds: None,
        };
        let bytes = r.encode();
        let decoded = BindResponse::decode(&bytes).unwrap();
        assert_eq!(decoded, r);
        assert_eq!(BindOutcome::from_response(&decoded), BindOutcome::Success);
    }

    #[test]
    fn bind_response_invalid_credentials_round_trip() {
        let r = BindResponse {
            message_id: 2,
            result_code: result::INVALID_CREDENTIALS,
            matched_dn: String::new(),
            diagnostic_message: "data 52e".into(),
            server_sasl_creds: None,
        };
        let bytes = r.encode();
        let decoded = BindResponse::decode(&bytes).unwrap();
        assert_eq!(decoded.result_code, 49);
        assert!(matches!(BindOutcome::from_response(&decoded), BindOutcome::InvalidCredentials));
    }

    #[test]
    fn bind_outcome_other_maps_unknown_codes() {
        let r = BindResponse {
            message_id: 1,
            result_code: 53,
            matched_dn: String::new(),
            diagnostic_message: "unwillingToPerform".into(),
            server_sasl_creds: None,
        };
        match BindOutcome::from_response(&r) {
            BindOutcome::Other { code, message } => {
                assert_eq!(code, 53);
                assert_eq!(message, "unwillingToPerform");
            }
            _ => panic!("expected Other"),
        }
    }
}

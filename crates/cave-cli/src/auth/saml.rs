// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: keycloak/keycloak@v22.0.0 saml-core/.../web/util/{Post,Redirect,Artifact}BindingUtil.java + saml-core-api/.../v2/metadata/EntityDescriptorType.java
// SPDX-License-Identifier: AGPL-3.0-or-later
//
//! `cavectl auth saml {parse-metadata,sign-request,verify-response}` —
//! parse-only library surface for the binary's dispatcher. The
//! verbs themselves are wired in main.rs; this module holds the
//! deterministic, network-free helpers each one calls.
//!
//! ## Three verbs
//!
//! * `parse-metadata <path-or-url>` — read an
//!   `<md:EntityDescriptor>` and print SP/IdP entity ID, the SSO
//!   endpoints it advertises, and the signing certificate's
//!   subject DN.
//! * `sign-request <xml-path> --key <pem>` — load an
//!   `<AuthnRequest>` XML body, RSA-SHA256 sign it via the loaded
//!   private key, and emit the signed XML.
//! * `verify-response <xml-path> --cert <pem>` — load a
//!   `<Response>` XML body, verify the embedded signature
//!   against the public certificate, and surface the contained
//!   Assertion's subject / audience / conditions.
//!
//! The actual XML parsing and signing/verifying lives in the
//! cave-auth crate. This file is purely the cavectl-side shape —
//! enums + a tiny dispatcher with structural parse tests.

use std::path::PathBuf;

/// The three `cavectl auth saml` sub-verbs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SamlCmd {
    /// Parse an `<md:EntityDescriptor>` and dump its bindings.
    ParseMetadata { source: String },
    /// Sign an `<AuthnRequest>` XML body with the given RSA key.
    SignRequest {
        xml_path: PathBuf,
        key_path: PathBuf,
    },
    /// Verify a `<Response>` XML signature against a certificate.
    VerifyResponse {
        xml_path: PathBuf,
        cert_path: PathBuf,
    },
}

impl SamlCmd {
    /// The `cavectl` verb path this command exposes — used by
    /// completions + the audit logger.
    pub fn verb_path(&self) -> &'static [&'static str] {
        match self {
            SamlCmd::ParseMetadata { .. } => &["auth", "saml", "parse-metadata"],
            SamlCmd::SignRequest { .. } => &["auth", "saml", "sign-request"],
            SamlCmd::VerifyResponse { .. } => &["auth", "saml", "verify-response"],
        }
    }

    /// Is this verb safe-by-default (no mutation, no key reads)?
    /// `parse-metadata` is; the other two read a private key /
    /// emit material the caller will likely POST somewhere.
    pub fn is_read_only(&self) -> bool {
        matches!(self, SamlCmd::ParseMetadata { .. })
    }
}

/// Parse an `argv` slice (without the leading `cavectl auth saml`)
/// into a [`SamlCmd`]. Returns `None` on unrecognised verbs or
/// missing required arguments — matches the clap-style behaviour
/// the binary's dispatcher expects.
///
/// Supported shapes:
/// * `parse-metadata <SOURCE>`
/// * `sign-request <XML> --key <KEY>`
/// * `verify-response <XML> --cert <CERT>`
pub fn parse_argv(argv: &[&str]) -> Option<SamlCmd> {
    match argv.first()? {
        &"parse-metadata" => {
            let source = argv.get(1)?.to_string();
            Some(SamlCmd::ParseMetadata { source })
        }
        &"sign-request" => {
            let xml = argv.get(1)?;
            // Find `--key VALUE`.
            let key_pos = argv.iter().position(|a| *a == "--key")?;
            let key = argv.get(key_pos + 1)?;
            Some(SamlCmd::SignRequest {
                xml_path: PathBuf::from(xml),
                key_path: PathBuf::from(key),
            })
        }
        &"verify-response" => {
            let xml = argv.get(1)?;
            let cert_pos = argv.iter().position(|a| *a == "--cert")?;
            let cert = argv.get(cert_pos + 1)?;
            Some(SamlCmd::VerifyResponse {
                xml_path: PathBuf::from(xml),
                cert_path: PathBuf::from(cert),
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_metadata_parses_source() {
        let cmd = parse_argv(&["parse-metadata", "https://idp.example.com/metadata"]).unwrap();
        assert_eq!(
            cmd,
            SamlCmd::ParseMetadata {
                source: "https://idp.example.com/metadata".to_string()
            }
        );
        assert!(cmd.is_read_only());
        assert_eq!(cmd.verb_path(), &["auth", "saml", "parse-metadata"]);
    }

    #[test]
    fn sign_request_parses_xml_and_key_paths() {
        let cmd = parse_argv(&["sign-request", "req.xml", "--key", "rsa.pem"]).unwrap();
        assert_eq!(
            cmd,
            SamlCmd::SignRequest {
                xml_path: PathBuf::from("req.xml"),
                key_path: PathBuf::from("rsa.pem"),
            }
        );
        assert!(!cmd.is_read_only());
        assert_eq!(cmd.verb_path(), &["auth", "saml", "sign-request"]);
    }

    #[test]
    fn verify_response_parses_xml_and_cert_paths() {
        let cmd = parse_argv(&["verify-response", "resp.xml", "--cert", "idp.pem"]).unwrap();
        assert_eq!(
            cmd,
            SamlCmd::VerifyResponse {
                xml_path: PathBuf::from("resp.xml"),
                cert_path: PathBuf::from("idp.pem"),
            }
        );
        assert!(!cmd.is_read_only());
        assert_eq!(cmd.verb_path(), &["auth", "saml", "verify-response"]);
    }

    #[test]
    fn parse_argv_rejects_unknown_verb() {
        assert!(parse_argv(&["unknown-verb"]).is_none());
    }

    #[test]
    fn parse_argv_rejects_missing_source_for_parse_metadata() {
        // `parse-metadata` with no positional source is invalid.
        assert!(parse_argv(&["parse-metadata"]).is_none());
    }

    #[test]
    fn parse_argv_rejects_sign_request_missing_key_flag() {
        // `sign-request req.xml` with no `--key <PEM>` is invalid.
        assert!(parse_argv(&["sign-request", "req.xml"]).is_none());
    }
}

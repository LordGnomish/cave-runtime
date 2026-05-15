// SPDX-License-Identifier: AGPL-3.0-or-later
//! SAML 2.0 NameID format negotiation between the SP's request
//! (`<samlp:NameIDPolicy Format=…>`) and the IdP's supported-set.
//!
//! Source: keycloak/keycloak@b825ba97
//!         services/src/main/java/org/keycloak/broker/saml/SAMLEndpoint.java::createNameId
//!         services/src/main/java/org/keycloak/protocol/saml/SamlProtocol.java::getSamlNameId
//!
//! Per SAML 2.0 Core §3.4.1.1, if the SP's `<NameIDPolicy>` Format is set
//! and the IdP cannot satisfy it, the IdP must return
//! `urn:oasis:names:tc:SAML:2.0:status:InvalidNameIDPolicy`. cave-auth
//! models this with a [`NameIdPolicyOutcome`] enum so the caller can
//! decide whether to mint an Assertion (`Granted`) or a Status-only
//! Response (`InvalidPolicy`).

use super::NameIdFormat;

/// Which NameID formats this IdP can produce, plus the format the IdP
/// picks when the SP leaves the choice open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameIdSupport {
    /// Set of formats the IdP can mint.
    pub supported: Vec<NameIdFormat>,
    /// Format used when the SP omits `<samlp:NameIDPolicy>` or sets it
    /// to `Unspecified`.
    pub default: NameIdFormat,
}

impl NameIdSupport {
    /// IdP that supports all four canonical formats; sensible default
    /// for cave-auth's IdP role. (`Unspecified` is treated as a
    /// real format here even though SP requests for it usually mean
    /// "anything you like".)
    pub fn all() -> Self {
        Self {
            supported: vec![
                NameIdFormat::EmailAddress,
                NameIdFormat::Persistent,
                NameIdFormat::Transient,
                NameIdFormat::Unspecified,
            ],
            default: NameIdFormat::EmailAddress,
        }
    }
}

/// Outcome of the negotiation, carrying either the granted Format or
/// the spec's `InvalidNameIDPolicy` status code reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NameIdPolicyOutcome {
    /// IdP accepts the format and will emit a NameID with it.
    Granted(NameIdFormat),
    /// IdP cannot satisfy the SP's `<NameIDPolicy>` — the resulting
    /// `<samlp:Response>` carries an `InvalidNameIDPolicy` sub-status.
    InvalidPolicy {
        requested: NameIdFormat,
        supported: Vec<NameIdFormat>,
    },
}

/// `urn:oasis:names:tc:SAML:2.0:status:InvalidNameIDPolicy` — the
/// secondary `<samlp:StatusCode>` an IdP returns when it rejects the
/// SP's requested NameID format.
pub const STATUS_INVALID_NAMEID_POLICY: &str =
    "urn:oasis:names:tc:SAML:2.0:status:InvalidNameIDPolicy";

/// Decide which NameID Format to mint. If the SP requests one the IdP
/// can satisfy, return it as `Granted`. If the SP requests one the IdP
/// can't satisfy (and didn't ask for `Unspecified`), return
/// `InvalidPolicy`. If the SP omitted `NameIDPolicy`, return the IdP's
/// default.
pub fn negotiate_nameid_format(
    requested: Option<NameIdFormat>,
    idp: &NameIdSupport,
) -> NameIdPolicyOutcome {
    match requested {
        None | Some(NameIdFormat::Unspecified) => {
            // SP gives the IdP free choice. We use the configured default
            // if it's supported, otherwise fall back to the first supported.
            if idp.supported.contains(&idp.default) {
                NameIdPolicyOutcome::Granted(idp.default)
            } else if let Some(first) = idp.supported.first() {
                NameIdPolicyOutcome::Granted(*first)
            } else {
                NameIdPolicyOutcome::InvalidPolicy {
                    requested: NameIdFormat::Unspecified,
                    supported: idp.supported.clone(),
                }
            }
        }
        Some(want) => {
            if idp.supported.contains(&want) {
                NameIdPolicyOutcome::Granted(want)
            } else {
                NameIdPolicyOutcome::InvalidPolicy {
                    requested: want,
                    supported: idp.supported.clone(),
                }
            }
        }
    }
}

/// Render a `<saml:NameID Format="…">value</saml:NameID>` fragment.
/// XML-escapes the body to prevent injection. The element is emitted
/// even when the format is `Unspecified` so callers can splice this
/// into a larger Assertion without conditional logic.
pub fn render_nameid(value: &str, format: NameIdFormat) -> String {
    let escaped: String = value
        .chars()
        .map(|c| match c {
            '<' => "&lt;".to_string(),
            '>' => "&gt;".to_string(),
            '&' => "&amp;".to_string(),
            '"' => "&quot;".to_string(),
            '\'' => "&apos;".to_string(),
            other => other.to_string(),
        })
        .collect();
    format!(
        r#"<saml:NameID xmlns:saml="urn:oasis:names:tc:SAML:2.0:assertion" Format="{fmt}">{val}</saml:NameID>"#,
        fmt = format.as_urn(),
        val = escaped,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_nameid_escapes_ampersand() {
        let xml = render_nameid("a&b", NameIdFormat::EmailAddress);
        assert!(xml.contains("a&amp;b"));
        assert!(!xml.contains("a&b<"));
    }

    #[test]
    fn negotiate_with_no_support_returns_invalid_policy() {
        let idp = NameIdSupport {
            supported: Vec::new(),
            default: NameIdFormat::EmailAddress,
        };
        let out = negotiate_nameid_format(None, &idp);
        assert!(matches!(out, NameIdPolicyOutcome::InvalidPolicy { .. }));
    }

    #[test]
    fn negotiate_unspecified_request_falls_back_to_default() {
        let idp = NameIdSupport {
            supported: vec![NameIdFormat::Persistent, NameIdFormat::Transient],
            default: NameIdFormat::Persistent,
        };
        let out = negotiate_nameid_format(Some(NameIdFormat::Unspecified), &idp);
        assert_eq!(out, NameIdPolicyOutcome::Granted(NameIdFormat::Persistent));
    }
}

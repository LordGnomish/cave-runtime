// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: keycloak/keycloak@v22.0.0 saml-core-api/src/main/java/org/keycloak/dom/saml/v2/protocol/NameIDPolicyType.java + saml-core-api/.../NameIDType.java
// SPDX-License-Identifier: AGPL-3.0-or-later
//
//! SAML 2.0 `<samlp:NameIDPolicy>` + `<saml:NameID>` helpers.
//!
//! `NameIDPolicy` lives on `AuthnRequest` and tells the IdP what
//! kind of subject identifier the SP wants back. `NameID` is the
//! identifier itself, carried inside the Assertion's Subject. The
//! existing [`super::NameIdFormat`] enum names the formats; this
//! module adds the surrounding scaffolding Keycloak ships in
//! `NameIDPolicyType.java` (allow-create, sp-name-qualifier,
//! name-qualifier round-trips).
//!
//! Why a separate module from `mod.rs`? Keycloak factors these
//! into `saml-core-api/.../v2/protocol/NameIDPolicyType` and
//! `saml-core-api/.../v2/assertion/NameIDType` — the request side
//! is *policy* (what the SP wants), the assertion side is the
//! *identifier itself* (what the IdP returned). Same enum
//! underneath; different surrounding fields.

use super::NameIdFormat;

/// Mirror of Keycloak's `NameIDPolicyType` — the `<samlp:NameIDPolicy>`
/// child element of `<samlp:AuthnRequest>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameIdPolicy {
    /// Format URN — what kind of identifier the SP is asking for.
    pub format: NameIdFormat,
    /// `SPNameQualifier` — namespaces persistent identifiers per-SP.
    /// `None` means the SP entity ID is implicit.
    pub sp_name_qualifier: Option<String>,
    /// `AllowCreate` — may the IdP mint a new identifier if the
    /// principal doesn't already have one? Default `true` for new
    /// federations; `false` for opt-in flows.
    pub allow_create: bool,
}

impl NameIdPolicy {
    /// New policy with `AllowCreate=true`.
    pub fn new(format: NameIdFormat) -> Self {
        NameIdPolicy {
            format,
            sp_name_qualifier: None,
            allow_create: true,
        }
    }

    /// Builder — set `SPNameQualifier`.
    pub fn with_sp_name_qualifier(mut self, qualifier: impl Into<String>) -> Self {
        self.sp_name_qualifier = Some(qualifier.into());
        self
    }

    /// Builder — set `AllowCreate=false`.
    pub fn deny_create(mut self) -> Self {
        self.allow_create = false;
        self
    }

    /// Render this policy as the inner XML of a
    /// `<samlp:NameIDPolicy ... />` element (self-closing).
    pub fn to_xml_fragment(&self) -> String {
        let mut out = String::from("<samlp:NameIDPolicy");
        out.push_str(&format!(" Format=\"{}\"", self.format.as_urn()));
        if let Some(q) = &self.sp_name_qualifier {
            out.push_str(&format!(" SPNameQualifier=\"{}\"", xml_escape(q)));
        }
        out.push_str(&format!(
            " AllowCreate=\"{}\"",
            if self.allow_create { "true" } else { "false" }
        ));
        out.push_str("/>");
        out
    }
}

/// Fully-qualified `<saml:NameID>` — value plus the qualifier set
/// the issuer scoped it under. Mirrors `NameIDType` from
/// `saml-core-api`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameId {
    /// The identifier value itself — opaque-or-email per format.
    pub value: String,
    /// Format URN.
    pub format: NameIdFormat,
    /// `NameQualifier` — issuer-side namespace, almost always the
    /// IdP entity ID.
    pub name_qualifier: Option<String>,
    /// `SPNameQualifier` — relying-party-side namespace, almost
    /// always the SP entity ID.
    pub sp_name_qualifier: Option<String>,
}

impl NameId {
    /// New NameID with the given format and no qualifiers.
    pub fn new(value: impl Into<String>, format: NameIdFormat) -> Self {
        NameId {
            value: value.into(),
            format,
            name_qualifier: None,
            sp_name_qualifier: None,
        }
    }

    /// Builder — set `NameQualifier`.
    pub fn with_name_qualifier(mut self, q: impl Into<String>) -> Self {
        self.name_qualifier = Some(q.into());
        self
    }

    /// Builder — set `SPNameQualifier`.
    pub fn with_sp_name_qualifier(mut self, q: impl Into<String>) -> Self {
        self.sp_name_qualifier = Some(q.into());
        self
    }

    /// Render as `<saml:NameID ... >value</saml:NameID>`.
    pub fn to_xml(&self) -> String {
        let mut out = String::from("<saml:NameID");
        out.push_str(&format!(" Format=\"{}\"", self.format.as_urn()));
        if let Some(q) = &self.name_qualifier {
            out.push_str(&format!(" NameQualifier=\"{}\"", xml_escape(q)));
        }
        if let Some(q) = &self.sp_name_qualifier {
            out.push_str(&format!(" SPNameQualifier=\"{}\"", xml_escape(q)));
        }
        out.push('>');
        out.push_str(&xml_escape(&self.value));
        out.push_str("</saml:NameID>");
        out
    }

    /// Equality up to canonical form — two NameIDs that refer to
    /// the same principal under the same qualifier set. SAML §8.3.7:
    /// value + format must match; qualifiers must agree (both
    /// absent or both present and equal).
    pub fn matches(&self, other: &NameId) -> bool {
        self.value == other.value
            && self.format == other.format
            && self.name_qualifier == other.name_qualifier
            && self.sp_name_qualifier == other.sp_name_qualifier
    }
}

/// XML attribute escaper — minimal but correct for the five
/// reserved characters. `quick-xml` does this for elements; we
/// hand-roll because these fragments are concatenated.
fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_id_policy_new_defaults_allow_create_true() {
        let p = NameIdPolicy::new(NameIdFormat::Persistent);
        assert!(p.allow_create);
        assert_eq!(p.format, NameIdFormat::Persistent);
        assert!(p.sp_name_qualifier.is_none());
    }

    #[test]
    fn name_id_policy_builder_deny_create() {
        let p = NameIdPolicy::new(NameIdFormat::Transient).deny_create();
        assert!(!p.allow_create);
    }

    #[test]
    fn name_id_policy_xml_includes_format_and_allow_create() {
        let p = NameIdPolicy::new(NameIdFormat::EmailAddress);
        let xml = p.to_xml_fragment();
        assert!(xml.contains("Format=\""));
        assert!(xml.contains("emailAddress"));
        assert!(xml.contains("AllowCreate=\"true\""));
    }

    #[test]
    fn name_id_policy_xml_includes_sp_qualifier_when_set() {
        let p = NameIdPolicy::new(NameIdFormat::Persistent)
            .with_sp_name_qualifier("https://sp.example.com/saml");
        let xml = p.to_xml_fragment();
        assert!(xml.contains("SPNameQualifier=\"https://sp.example.com/saml\""));
    }

    #[test]
    fn name_id_policy_xml_self_closes() {
        let p = NameIdPolicy::new(NameIdFormat::Persistent);
        let xml = p.to_xml_fragment();
        assert!(xml.ends_with("/>"));
    }

    #[test]
    fn name_id_new_has_no_qualifiers() {
        let n = NameId::new("user@example.com", NameIdFormat::EmailAddress);
        assert_eq!(n.value, "user@example.com");
        assert!(n.name_qualifier.is_none());
        assert!(n.sp_name_qualifier.is_none());
    }

    #[test]
    fn name_id_to_xml_round_trips_basic() {
        let n = NameId::new("user@example.com", NameIdFormat::EmailAddress);
        let xml = n.to_xml();
        assert!(xml.starts_with("<saml:NameID"));
        assert!(xml.ends_with("</saml:NameID>"));
        assert!(xml.contains("user@example.com"));
        assert!(xml.contains("emailAddress"));
    }

    #[test]
    fn name_id_xml_escapes_reserved_chars() {
        let n = NameId::new("user&<weird>\"", NameIdFormat::Unspecified);
        let xml = n.to_xml();
        assert!(xml.contains("&amp;"));
        assert!(xml.contains("&lt;"));
        assert!(xml.contains("&gt;"));
        assert!(xml.contains("&quot;"));
        assert!(!xml.contains("user&<"));
    }

    #[test]
    fn name_id_matches_reflexive() {
        let a = NameId::new("x", NameIdFormat::Persistent)
            .with_name_qualifier("idp")
            .with_sp_name_qualifier("sp");
        assert!(a.matches(&a.clone()));
    }

    #[test]
    fn name_id_matches_rejects_qualifier_mismatch() {
        let a = NameId::new("x", NameIdFormat::Persistent).with_sp_name_qualifier("sp-a");
        let b = NameId::new("x", NameIdFormat::Persistent).with_sp_name_qualifier("sp-b");
        assert!(!a.matches(&b));
    }

    #[test]
    fn name_id_matches_rejects_name_qualifier_mismatch() {
        let a = NameId::new("u", NameIdFormat::Persistent).with_name_qualifier("idp-A");
        let b = NameId::new("u", NameIdFormat::Persistent).with_name_qualifier("idp-B");
        assert!(!a.matches(&b));
    }

    #[test]
    fn name_id_matches_rejects_format_mismatch() {
        let a = NameId::new("x", NameIdFormat::Persistent);
        let b = NameId::new("x", NameIdFormat::Transient);
        assert!(!a.matches(&b));
    }

    #[test]
    fn name_id_matches_rejects_value_mismatch() {
        let a = NameId::new("alice", NameIdFormat::EmailAddress);
        let b = NameId::new("bob", NameIdFormat::EmailAddress);
        assert!(!a.matches(&b));
    }

    #[test]
    fn name_id_with_builders_chain() {
        let n = NameId::new("u", NameIdFormat::Persistent)
            .with_name_qualifier("issuer")
            .with_sp_name_qualifier("relying-party");
        assert_eq!(n.name_qualifier.as_deref(), Some("issuer"));
        assert_eq!(n.sp_name_qualifier.as_deref(), Some("relying-party"));
    }
}

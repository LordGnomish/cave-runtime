// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: keycloak/keycloak@v22.0.0 saml-core-api/src/main/java/org/keycloak/dom/saml/v2/assertion/AssertionType.java + AuthnContextClassRefType.java + ConditionsType.java
// SPDX-License-Identifier: AGPL-3.0-or-later
//
//! SAML 2.0 Assertion-side helpers — Conditions evaluation,
//! AuthnContextClassRef enum, subject-confirmation method URNs.
//!
//! The full `<saml:Assertion>` parser already lives in
//! `super::response::Assertion`. This module adds the small but
//! load-bearing scaffolding around it: timing-window + audience
//! validation logic, the `AuthnContextClassRef` enum step-up MFA
//! decisions branch on, and the SubjectConfirmation method URNs
//! the Bearer profile pins.
//!
//! Why a separate module? Keycloak factors these into distinct
//! sibling types in `saml-core-api/.../v2/assertion/`. The
//! `AssertionType` proper is the container; `ConditionsType`,
//! `AuthnContextType`, and `SubjectConfirmationType` are reusable
//! independently — same factoring here.

use chrono::{DateTime, Utc};

/// `<saml:Conditions>` — `NotBefore` / `NotOnOrAfter` window plus
/// AudienceRestriction list. cave-auth flattens AudienceRestriction
/// children into a single `audiences: Vec<String>` (matches every
/// real-world IdP that emits exactly one AudienceRestriction).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssertionConditions {
    pub not_before: DateTime<Utc>,
    pub not_on_or_after: DateTime<Utc>,
    pub audiences: Vec<String>,
}

impl AssertionConditions {
    /// Is the supplied moment inside `[NotBefore, NotOnOrAfter)`?
    /// SAML 2.0 §2.5.1.2 — closed on the lower bound, open on the
    /// upper.
    pub fn is_time_valid(&self, at: DateTime<Utc>) -> bool {
        at >= self.not_before && at < self.not_on_or_after
    }

    /// Does the audience list contain `aud`? Used by the SP to
    /// verify it is in fact the intended consumer.
    pub fn audience_matches(&self, aud: &str) -> bool {
        self.audiences.iter().any(|a| a == aud)
    }

    /// New conditions with a window `[now, now+ttl)` and one
    /// audience.
    pub fn new(now: DateTime<Utc>, ttl_secs: i64, audience: impl Into<String>) -> Self {
        AssertionConditions {
            not_before: now,
            not_on_or_after: now + chrono::Duration::seconds(ttl_secs),
            audiences: vec![audience.into()],
        }
    }
}

/// `<saml:AuthnContextClassRef>` — the assurance level / mechanism
/// the IdP authenticated with. Step-up MFA flows branch on this.
///
/// SAML 2.0 names ~25 classes; the five below are the ones
/// Keycloak's `AuthnContextClassRefType` constants ship and the
/// only ones cave-auth has a use for. The catch-all `Unspecified`
/// is the spec's fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthnContextClass {
    /// Password sent over a protected transport (TLS). The most
    /// common value.
    PasswordProtectedTransport,
    /// Password over any transport — weaker than `PPT`.
    Password,
    /// Kerberos / SPNEGO.
    Kerberos,
    /// Previously-authenticated session.
    PreviousSession,
    /// Unspecified — used when the IdP doesn't know.
    Unspecified,
}

impl AuthnContextClass {
    /// SAML 2.0 URN for this class.
    pub fn as_urn(self) -> &'static str {
        match self {
            AuthnContextClass::PasswordProtectedTransport => {
                "urn:oasis:names:tc:SAML:2.0:ac:classes:PasswordProtectedTransport"
            }
            AuthnContextClass::Password => "urn:oasis:names:tc:SAML:2.0:ac:classes:Password",
            AuthnContextClass::Kerberos => "urn:oasis:names:tc:SAML:2.0:ac:classes:Kerberos",
            AuthnContextClass::PreviousSession => {
                "urn:oasis:names:tc:SAML:2.0:ac:classes:PreviousSession"
            }
            AuthnContextClass::Unspecified => "urn:oasis:names:tc:SAML:2.0:ac:classes:unspecified",
        }
    }

    /// Inverse of `as_urn` — `None` for unrecognised URNs.
    pub fn from_urn(s: &str) -> Option<Self> {
        match s {
            "urn:oasis:names:tc:SAML:2.0:ac:classes:PasswordProtectedTransport" => {
                Some(AuthnContextClass::PasswordProtectedTransport)
            }
            "urn:oasis:names:tc:SAML:2.0:ac:classes:Password" => Some(AuthnContextClass::Password),
            "urn:oasis:names:tc:SAML:2.0:ac:classes:Kerberos" => Some(AuthnContextClass::Kerberos),
            "urn:oasis:names:tc:SAML:2.0:ac:classes:PreviousSession" => {
                Some(AuthnContextClass::PreviousSession)
            }
            "urn:oasis:names:tc:SAML:2.0:ac:classes:unspecified" => {
                Some(AuthnContextClass::Unspecified)
            }
            _ => None,
        }
    }

    /// Numeric strength used for step-up comparisons. Higher is
    /// stronger. Heuristic — SAML 2.0 itself doesn't define a
    /// total ordering, but cave-auth's MFA broker treats
    /// `PreviousSession < Password < PPT < Kerberos`.
    pub fn strength(self) -> u8 {
        match self {
            AuthnContextClass::Unspecified => 0,
            AuthnContextClass::PreviousSession => 1,
            AuthnContextClass::Password => 2,
            AuthnContextClass::PasswordProtectedTransport => 3,
            AuthnContextClass::Kerberos => 4,
        }
    }
}

/// `<saml:SubjectConfirmation>` Method URNs. The Bearer profile
/// (the one Web SSO uses) pins `urn:oasis:names:tc:SAML:2.0:cm:bearer`;
/// the other two are less common but appear in
/// `SubjectConfirmationType.java` constants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubjectConfirmationMethod {
    /// Bearer — possession of the Assertion is sufficient.
    Bearer,
    /// Holder-of-Key — possession plus proof of a confirmation key.
    HolderOfKey,
    /// Sender-Vouches — issued by an attesting entity.
    SenderVouches,
}

impl SubjectConfirmationMethod {
    pub fn as_urn(self) -> &'static str {
        match self {
            SubjectConfirmationMethod::Bearer => "urn:oasis:names:tc:SAML:2.0:cm:bearer",
            SubjectConfirmationMethod::HolderOfKey => {
                "urn:oasis:names:tc:SAML:2.0:cm:holder-of-key"
            }
            SubjectConfirmationMethod::SenderVouches => {
                "urn:oasis:names:tc:SAML:2.0:cm:sender-vouches"
            }
        }
    }

    pub fn from_urn(s: &str) -> Option<Self> {
        match s {
            "urn:oasis:names:tc:SAML:2.0:cm:bearer" => Some(SubjectConfirmationMethod::Bearer),
            "urn:oasis:names:tc:SAML:2.0:cm:holder-of-key" => {
                Some(SubjectConfirmationMethod::HolderOfKey)
            }
            "urn:oasis:names:tc:SAML:2.0:cm:sender-vouches" => {
                Some(SubjectConfirmationMethod::SenderVouches)
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn t(offset_secs: i64) -> DateTime<Utc> {
        // A stable reference time used across tests — Unix epoch +
        // some offset. Replaces any wall-clock dependency.
        DateTime::<Utc>::from_timestamp(1_700_000_000 + offset_secs, 0).unwrap()
    }

    #[test]
    fn conditions_time_valid_inside_window() {
        let c = AssertionConditions {
            not_before: t(0),
            not_on_or_after: t(60),
            audiences: vec![],
        };
        assert!(c.is_time_valid(t(30)));
    }

    #[test]
    fn conditions_time_invalid_before_window() {
        let c = AssertionConditions {
            not_before: t(10),
            not_on_or_after: t(60),
            audiences: vec![],
        };
        assert!(!c.is_time_valid(t(0)));
    }

    #[test]
    fn conditions_time_invalid_at_upper_bound() {
        // NotOnOrAfter is an open upper bound — equality must
        // reject.
        let c = AssertionConditions {
            not_before: t(0),
            not_on_or_after: t(60),
            audiences: vec![],
        };
        assert!(!c.is_time_valid(t(60)));
    }

    #[test]
    fn conditions_time_valid_at_lower_bound() {
        let c = AssertionConditions {
            not_before: t(0),
            not_on_or_after: t(60),
            audiences: vec![],
        };
        // NotBefore is closed.
        assert!(c.is_time_valid(t(0)));
    }

    #[test]
    fn conditions_audience_match_single() {
        let c = AssertionConditions::new(t(0), 60, "https://sp.example.com");
        assert!(c.audience_matches("https://sp.example.com"));
        assert!(!c.audience_matches("https://other-sp.example.com"));
    }

    #[test]
    fn conditions_audience_match_multiple() {
        let c = AssertionConditions {
            not_before: t(0),
            not_on_or_after: t(60),
            audiences: vec!["a".to_string(), "b".to_string()],
        };
        assert!(c.audience_matches("a"));
        assert!(c.audience_matches("b"));
        assert!(!c.audience_matches("c"));
    }

    #[test]
    fn conditions_new_builds_window() {
        let c = AssertionConditions::new(t(0), 300, "aud");
        assert_eq!(c.not_before, t(0));
        assert_eq!(c.not_on_or_after - c.not_before, Duration::seconds(300));
        assert_eq!(c.audiences, vec!["aud".to_string()]);
    }

    #[test]
    fn authn_context_class_urn_roundtrip_all_variants() {
        for class in [
            AuthnContextClass::PasswordProtectedTransport,
            AuthnContextClass::Password,
            AuthnContextClass::Kerberos,
            AuthnContextClass::PreviousSession,
            AuthnContextClass::Unspecified,
        ] {
            assert_eq!(AuthnContextClass::from_urn(class.as_urn()), Some(class));
        }
    }

    #[test]
    fn authn_context_class_unknown_urn_is_none() {
        assert!(AuthnContextClass::from_urn("urn:invented").is_none());
    }

    #[test]
    fn authn_context_class_strength_order() {
        assert!(
            AuthnContextClass::Kerberos.strength()
                > AuthnContextClass::PasswordProtectedTransport.strength()
        );
        assert!(
            AuthnContextClass::PasswordProtectedTransport.strength()
                > AuthnContextClass::Password.strength()
        );
        assert!(
            AuthnContextClass::Password.strength() > AuthnContextClass::PreviousSession.strength()
        );
        assert!(
            AuthnContextClass::PreviousSession.strength()
                > AuthnContextClass::Unspecified.strength()
        );
    }

    #[test]
    fn subject_confirmation_method_roundtrip() {
        for m in [
            SubjectConfirmationMethod::Bearer,
            SubjectConfirmationMethod::HolderOfKey,
            SubjectConfirmationMethod::SenderVouches,
        ] {
            assert_eq!(SubjectConfirmationMethod::from_urn(m.as_urn()), Some(m));
        }
    }

    #[test]
    fn subject_confirmation_method_unknown_urn_is_none() {
        assert!(SubjectConfirmationMethod::from_urn("urn:other").is_none());
    }

    #[test]
    fn subject_confirmation_bearer_urn_is_correct_string() {
        // Wire-level pin — Bearer is the Web SSO profile URN.
        assert_eq!(
            SubjectConfirmationMethod::Bearer.as_urn(),
            "urn:oasis:names:tc:SAML:2.0:cm:bearer"
        );
    }
}

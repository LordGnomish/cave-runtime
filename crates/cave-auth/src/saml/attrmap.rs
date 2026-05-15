// SPDX-License-Identifier: AGPL-3.0-or-later
//! Map SAML `<saml:AttributeStatement>` claims onto Cave RBAC roles.
//!
//! Source: keycloak/keycloak@b825ba97
//!         services/src/main/java/org/keycloak/broker/saml/mappers/AttributeToRoleMapper.java
//!         services/src/main/java/org/keycloak/broker/saml/mappers/UserAttributeMapper.java
//!
//! Keycloak's broker layer plugs `IdentityProviderMapper` impls between
//! the verified SAML Assertion and the local realm. The most common one
//! is `AttributeToRoleMapper`: "if SAML attribute X carries value Y,
//! grant the realm role Z". cave-auth ports the same shape: an
//! `AttributeRoleMapping` is a (attribute, predicate, role) triple, and
//! `apply_mappings` evaluates them against a verified [`SamlSubject`].
//!
//! Cave's RBAC engine consumes role *names* (see `cave_auth::rbac::Role`),
//! so the mapper output is `Vec<String>` of role names. The auth_middleware
//! layer then resolves names through `RbacEngine::role_by_name` /
//! attaches the bindings.

use regex::Regex;

use super::SamlSubject;

/// One mapping rule: when SAML attribute `attribute_name` carries a
/// value matching `attribute_match`, grant `target_role`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttributeRoleMapping {
    /// Name of the SAML attribute (e.g. `memberOf`, `groups`, `Role`).
    pub attribute_name: String,
    /// Predicate that decides whether a particular attribute value
    /// triggers the role grant.
    pub attribute_match: AttributeMatch,
    /// Cave RBAC role name to grant when the predicate matches.
    pub target_role: String,
}

/// How a mapping decides whether a `<saml:AttributeValue>` matches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttributeMatch {
    /// Exact byte-for-byte string equality.
    ExactValue(String),
    /// `regex` crate pattern. Compiled per call; pre-compile your own
    /// `regex::Regex` if profile data flags this as hot.
    Regex(String),
    /// "Attribute is present with any value" — the cheapest predicate.
    /// Useful for "anyone with a verified `email` becomes `developer`".
    AnyValue,
}

/// Output of running a mapping set over a [`SamlSubject`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MapperOutput {
    /// Deduplicated set of role names the subject earned.
    pub granted_roles: Vec<String>,
    /// Attribute names that some mapping requested but the Assertion
    /// didn't carry. Surfaced to operators so misconfigured IdP exports
    /// are visible in the admin UI.
    pub unmatched: Vec<String>,
    /// Mapping evaluation errors (e.g. malformed regex). The mapper
    /// degrades open — a bad rule is logged here but doesn't crash the
    /// pipeline; the rest of the rules still run.
    pub errors: Vec<String>,
}

/// Evaluate `mappings` against `subject` and return the cumulative
/// `MapperOutput`. Order of `mappings` is irrelevant (granted_roles is
/// stable but dedup'd).
pub fn apply_mappings(subject: &SamlSubject, mappings: &[AttributeRoleMapping]) -> MapperOutput {
    let mut out = MapperOutput::default();

    for m in mappings {
        let values = match subject.attributes.get(&m.attribute_name) {
            Some(v) => v,
            None => {
                out.unmatched.push(m.attribute_name.clone());
                continue;
            }
        };
        let mut matched = false;
        match &m.attribute_match {
            AttributeMatch::ExactValue(expected) => {
                if values.iter().any(|v| v == expected) {
                    matched = true;
                }
            }
            AttributeMatch::AnyValue => {
                if !values.is_empty() {
                    matched = true;
                }
            }
            AttributeMatch::Regex(pat) => match Regex::new(pat) {
                Ok(re) => {
                    if values.iter().any(|v| re.is_match(v)) {
                        matched = true;
                    }
                }
                Err(e) => {
                    out.errors.push(format!("bad regex `{pat}`: {e}"));
                }
            },
        }
        if matched && !out.granted_roles.contains(&m.target_role) {
            out.granted_roles.push(m.target_role.clone());
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::saml::NameIdFormat;
    use std::collections::BTreeMap;

    fn empty_subject() -> SamlSubject {
        SamlSubject {
            name_id: "u".into(),
            name_id_format: NameIdFormat::Unspecified,
            issuer: "i".into(),
            attributes: BTreeMap::new(),
            session_index: None,
        }
    }

    #[test]
    fn empty_mappings_yields_empty_output() {
        let out = apply_mappings(&empty_subject(), &[]);
        assert!(out.granted_roles.is_empty());
        assert!(out.unmatched.is_empty());
        assert!(out.errors.is_empty());
    }

    #[test]
    fn any_value_does_not_match_empty_attribute_list() {
        let mut s = empty_subject();
        s.attributes
            .insert("groups".into(), vec![]); // present but empty list
        let m = AttributeRoleMapping {
            attribute_name: "groups".into(),
            attribute_match: AttributeMatch::AnyValue,
            target_role: "viewer".into(),
        };
        let out = apply_mappings(&s, &[m]);
        assert!(out.granted_roles.is_empty());
    }
}

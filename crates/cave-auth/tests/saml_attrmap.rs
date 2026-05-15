// SPDX-License-Identifier: AGPL-3.0-or-later
//! RED phase for SAML AttributeStatement → Cave RBAC role mapper.
//!
//! Source: keycloak/keycloak@b825ba97
//!         services/src/main/java/org/keycloak/broker/saml/mappers/AttributeToRoleMapper.java
//!         services/src/main/java/org/keycloak/broker/saml/mappers/UserAttributeMapper.java

use std::collections::BTreeMap;

use cave_auth::saml::attrmap::{
    apply_mappings, AttributeMatch, AttributeRoleMapping, MapperOutput,
};
use cave_auth::saml::{NameIdFormat, SamlSubject};

fn subject_with(attrs: &[(&str, &[&str])]) -> SamlSubject {
    let mut map: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (k, vs) in attrs {
        map.insert(
            k.to_string(),
            vs.iter().map(|s| s.to_string()).collect(),
        );
    }
    SamlSubject {
        name_id: "alice@example.com".into(),
        name_id_format: NameIdFormat::EmailAddress,
        issuer: "https://idp.example".into(),
        attributes: map,
        session_index: None,
    }
}

#[test]
fn exact_value_match_grants_role() {
    let mapping = AttributeRoleMapping {
        attribute_name: "memberOf".into(),
        attribute_match: AttributeMatch::ExactValue("cn=cave-admins,ou=groups".into()),
        target_role: "platform-admin".into(),
    };
    let s = subject_with(&[("memberOf", &["cn=cave-admins,ou=groups"])]);
    let out = apply_mappings(&s, &[mapping]);
    assert_eq!(out.granted_roles, vec!["platform-admin".to_string()]);
    assert!(out.unmatched.is_empty());
}

#[test]
fn missing_attribute_yields_no_role() {
    let mapping = AttributeRoleMapping {
        attribute_name: "memberOf".into(),
        attribute_match: AttributeMatch::ExactValue("cn=cave-admins".into()),
        target_role: "platform-admin".into(),
    };
    let s = subject_with(&[]); // no `memberOf` claim
    let out = apply_mappings(&s, &[mapping]);
    assert!(out.granted_roles.is_empty());
    assert_eq!(out.unmatched, vec!["memberOf".to_string()]);
}

#[test]
fn regex_match_grants_role() {
    let mapping = AttributeRoleMapping {
        attribute_name: "groups".into(),
        attribute_match: AttributeMatch::Regex("^cave-(admin|ops)$".into()),
        target_role: "module-admin".into(),
    };
    let s = subject_with(&[("groups", &["cave-ops", "other"])]);
    let out = apply_mappings(&s, &[mapping]);
    assert_eq!(out.granted_roles, vec!["module-admin".to_string()]);
}

#[test]
fn any_value_grants_when_attribute_present() {
    let mapping = AttributeRoleMapping {
        attribute_name: "email".into(),
        attribute_match: AttributeMatch::AnyValue,
        target_role: "developer".into(),
    };
    let s = subject_with(&[("email", &["a@b"])]);
    let out = apply_mappings(&s, &[mapping]);
    assert_eq!(out.granted_roles, vec!["developer".to_string()]);
}

#[test]
fn multiple_mappings_accumulate_distinct_roles() {
    let m1 = AttributeRoleMapping {
        attribute_name: "groups".into(),
        attribute_match: AttributeMatch::ExactValue("dev".into()),
        target_role: "developer".into(),
    };
    let m2 = AttributeRoleMapping {
        attribute_name: "groups".into(),
        attribute_match: AttributeMatch::ExactValue("auditor".into()),
        target_role: "auditor".into(),
    };
    let s = subject_with(&[("groups", &["dev", "auditor", "other"])]);
    let out: MapperOutput = apply_mappings(&s, &[m1, m2]);
    assert!(out.granted_roles.contains(&"developer".to_string()));
    assert!(out.granted_roles.contains(&"auditor".to_string()));
    assert_eq!(out.granted_roles.len(), 2, "dedup, no double-grant");
}

#[test]
fn invalid_regex_falls_through_to_unmatched() {
    let mapping = AttributeRoleMapping {
        attribute_name: "g".into(),
        attribute_match: AttributeMatch::Regex("[invalid(".into()),
        target_role: "viewer".into(),
    };
    let s = subject_with(&[("g", &["v"])]);
    let out = apply_mappings(&s, &[mapping]);
    assert!(out.granted_roles.is_empty());
    assert!(!out.errors.is_empty(), "captures bad regex via errors");
}

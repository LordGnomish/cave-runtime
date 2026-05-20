// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 federation/ldap/src/test/.../

//! Upstream-test port. Each `#[test]` here mirrors a Keycloak
//! JUnit fixture, named for traceability. The behaviours
//! checked are the ones Keycloak's `LDAPProvidersIntegrationTest`,
//! `LDAPGroupMapperTest`, and `MSADUserAccountControlStorageMapperTest`
//! lock down.

use crate::ldap::{
    active_directory::{UacFlag, UserAccountControl, parse_object_sid, pwd_last_set_to_unix_epoch},
    group_mapper::{GroupEntry, GroupMapper},
    query::{Filter, LdapQueryBuilder, Scope},
    storage_provider::{InMemoryDirectory, LdapStorageConfig, UserStorageProvider},
    user_mapper::UserAttributeMapper,
};
use std::collections::BTreeMap;

fn map_of(pairs: &[(&str, &[&str])]) -> BTreeMap<String, Vec<String>> {
    pairs
        .iter()
        .map(|(k, vs)| {
            (
                (*k).to_string(),
                vs.iter().map(|s| (*s).to_string()).collect(),
            )
        })
        .collect()
}

#[test]
fn ldap_providers_integration_test_search_user_by_username() {
    let mut d = InMemoryDirectory::new();
    d.insert(
        "uid=mary,ou=people,dc=example,dc=com",
        map_of(&[
            ("uid", &["mary"]),
            ("mail", &["mary@example.com"]),
            ("cn", &["Mary Kelly"]),
            ("objectClass", &["inetOrgPerson"]),
        ]),
    );
    let u = d.find_by_username("mary").unwrap().unwrap();
    assert_eq!(u.email.as_deref(), Some("mary@example.com"));
}

#[test]
fn ldap_providers_integration_test_search_user_returns_none_when_missing() {
    let d = InMemoryDirectory::new();
    assert!(d.find_by_username("ghost").unwrap().is_none());
}

#[test]
fn ldap_query_builder_default_scope_is_subtree() {
    let q = LdapQueryBuilder::new("dc=example,dc=com").build();
    assert_eq!(q.scope, Scope::Subtree);
}

#[test]
fn ldap_query_builder_collapses_zero_filters_to_present_objectclass() {
    let q = LdapQueryBuilder::new("dc=example,dc=com").build();
    match q.filter {
        Filter::Present { attr } => assert_eq!(attr, "objectClass"),
        _ => panic!("expected fallback to (objectClass=*)"),
    }
}

#[test]
fn ldap_group_mapper_test_member_of_strategy_reads_user_attr() {
    let m = GroupMapper::member_of_default();
    let groups = m.groups_of_user(&map_of(&[(
        "memberOf",
        &["cn=engineers,ou=groups,dc=example,dc=com"],
    )]));
    assert_eq!(groups, vec!["engineers"]);
}

#[test]
fn ldap_group_mapper_test_group_member_strategy_reads_group_attr() {
    let m = GroupMapper::group_member_default();
    let groups = vec![GroupEntry {
        dn: "cn=engineers,ou=groups".into(),
        name: "engineers".into(),
        members: vec!["uid=jdoe,ou=people".into()],
    }];
    let inv = m.invert_group_members(&groups);
    assert_eq!(
        inv.get("uid=jdoe,ou=people").unwrap(),
        &vec!["engineers".to_string()]
    );
}

#[test]
fn ldap_user_mapper_test_keycloak_default_attribute_map() {
    let m = UserAttributeMapper::keycloak_defaults();
    assert!(
        m.rows
            .iter()
            .any(|r| r.user_field == "username" && r.ldap_attr == "uid")
    );
    assert!(
        m.rows
            .iter()
            .any(|r| r.user_field == "email" && r.ldap_attr == "mail")
    );
    assert!(
        m.rows
            .iter()
            .any(|r| r.user_field == "displayName" && r.ldap_attr == "cn")
    );
}

#[test]
fn msad_user_account_control_storage_mapper_test_disabled_flag() {
    // Keycloak's `MSADUserAccountControlStorageMapperTest::testIsUserDisabled`
    let uac = UserAccountControl::parse("514").unwrap();
    assert!(uac.is_disabled());
}

#[test]
fn msad_user_account_control_storage_mapper_test_password_expired_flag() {
    let uac = UserAccountControl(0x800000 | 0x200);
    assert!(uac.password_expired());
}

#[test]
fn msad_user_account_control_storage_mapper_test_dont_expire_password() {
    let uac = UserAccountControl::parse("66048").unwrap();
    assert!(uac.never_expires());
}

#[test]
fn msad_user_account_control_storage_mapper_test_account_is_locked() {
    let uac = UserAccountControl(UacFlag::Lockout as u32);
    assert!(uac.is_locked());
}

#[test]
fn msad_object_sid_parser_test_builtin_administrators_sid() {
    // Equivalent to Keycloak's `LDAPUtils.decodeSid()` test for
    // S-1-5-32-544.
    let bytes = [
        0x01, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x05, 0x20, 0x00, 0x00, 0x00, 0x20, 0x02, 0x00,
        0x00,
    ];
    assert_eq!(parse_object_sid(&bytes).as_deref(), Some("S-1-5-32-544"));
}

#[test]
fn msad_pwd_last_set_parser_test_zero_means_must_change() {
    assert_eq!(pwd_last_set_to_unix_epoch("0"), Some(0));
}

#[test]
fn ldap_provider_active_directory_config_uses_samaccountname() {
    let c = LdapStorageConfig::active_directory_default("dc=example,dc=com");
    let row = c
        .user_mapper
        .rows
        .iter()
        .find(|r| r.user_field == "username")
        .unwrap();
    assert_eq!(row.ldap_attr, "sAMAccountName");
}

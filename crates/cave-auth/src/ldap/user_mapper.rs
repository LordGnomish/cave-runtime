// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 federation/ldap/src/main/java/org/keycloak/storage/ldap/mappers/UserAttributeLDAPStorageMapper.java

//! LDAP attribute → cave-auth user-field mapping. Port of
//! Keycloak's `UserAttributeLDAPStorageMapper`. The mapper is a
//! direction-aware table — one row per cave-auth user field +
//! its LDAP attribute alias + a value-shape rule (single /
//! multivalued / case-insensitive).

use std::collections::BTreeMap;

/// One row in the attribute map. Mirrors Keycloak's per-mapper
/// `ldap.attribute` / `user.model.attribute` / `is.binary.attribute`
/// config keys.
#[derive(Debug, Clone)]
pub struct AttributeMap {
    /// Cave user-model field name (e.g. `username`, `email`).
    pub user_field: String,
    /// LDAP attribute name (e.g. `uid`, `mail`).
    pub ldap_attr: String,
    /// When true, only the first LDAP value is kept (Keycloak's
    /// "Single Value" toggle).
    pub single_valued: bool,
    /// When true, compare values case-insensitively (matches
    /// RFC 4517 §3 `caseIgnoreMatch`).
    pub case_ignore: bool,
}

impl AttributeMap {
    pub fn new(
        user_field: impl Into<String>,
        ldap_attr: impl Into<String>,
        single_valued: bool,
        case_ignore: bool,
    ) -> Self {
        AttributeMap {
            user_field: user_field.into(),
            ldap_attr: ldap_attr.into(),
            single_valued,
            case_ignore,
        }
    }
}

/// cave-auth user view produced from an LDAP entry.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LdapUser {
    pub dn: String,
    pub username: String,
    pub email: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub display_name: Option<String>,
    pub groups: Vec<String>,
    /// Arbitrary attributes carried over verbatim (Keycloak
    /// surfaces these as `UserModel.getAttribute`).
    pub other_attrs: BTreeMap<String, Vec<String>>,
}

/// Aggregator — owns the attribute-map table and applies it to
/// LDAP entries. Equivalent to a tied bundle of Keycloak's
/// per-attribute mapper instances.
#[derive(Debug, Clone, Default)]
pub struct UserAttributeMapper {
    pub rows: Vec<AttributeMap>,
}

impl UserAttributeMapper {
    /// Keycloak's default mapper set — `uid → username`,
    /// `mail → email`, `givenName → firstName`, `sn → lastName`,
    /// `cn → displayName`, `memberOf → groups`.
    pub fn keycloak_defaults() -> Self {
        UserAttributeMapper {
            rows: vec![
                AttributeMap::new("username", "uid", true, true),
                AttributeMap::new("email", "mail", true, true),
                AttributeMap::new("firstName", "givenName", true, false),
                AttributeMap::new("lastName", "sn", true, false),
                AttributeMap::new("displayName", "cn", true, false),
                AttributeMap::new("groups", "memberOf", false, true),
            ],
        }
    }

    /// Apply this mapper to a search entry (attribute name →
    /// list of values).
    pub fn map_entry(
        &self,
        dn: &str,
        attrs: &BTreeMap<String, Vec<String>>,
    ) -> LdapUser {
        let mut user = LdapUser {
            dn: dn.to_owned(),
            ..Default::default()
        };
        let mut consumed = std::collections::BTreeSet::new();
        for row in &self.rows {
            // case-insensitive attribute lookup
            let entry = attrs.iter().find(|(k, _)| {
                if row.case_ignore {
                    k.eq_ignore_ascii_case(&row.ldap_attr)
                } else {
                    *k == &row.ldap_attr
                }
            });
            let Some((key, values)) = entry else { continue };
            consumed.insert(key.clone());
            if values.is_empty() {
                continue;
            }
            match row.user_field.as_str() {
                "username" => user.username = values[0].clone(),
                "email" => user.email = Some(values[0].clone()),
                "firstName" => user.first_name = Some(values[0].clone()),
                "lastName" => user.last_name = Some(values[0].clone()),
                "displayName" => user.display_name = Some(values[0].clone()),
                "groups" => {
                    user.groups = values
                        .iter()
                        .map(|dn| {
                            extract_cn_from_dn(dn)
                                .unwrap_or_else(|| dn.clone())
                        })
                        .collect();
                }
                other => {
                    let v = if row.single_valued {
                        vec![values[0].clone()]
                    } else {
                        values.clone()
                    };
                    user.other_attrs.insert(other.to_owned(), v);
                }
            }
        }
        // Surface any LDAP-side attrs not covered by a mapping
        // row — Keycloak does the same for `additional attributes`.
        for (k, v) in attrs {
            if !consumed.contains(k) {
                user.other_attrs.insert(k.clone(), v.clone());
            }
        }
        user
    }
}

/// Lift a CN-style group name out of a fully-qualified DN —
/// `cn=engineers,ou=groups,dc=example,dc=com` → `engineers`.
/// Returns `None` if the DN doesn't start with `cn=`.
pub fn extract_cn_from_dn(dn: &str) -> Option<String> {
    let head = dn.split(',').next()?.trim();
    let lower = head.to_ascii_lowercase();
    if lower.starts_with("cn=") {
        Some(head[3..].to_owned())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(pairs: &[(&str, &[&str])]) -> BTreeMap<String, Vec<String>> {
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
    fn maps_uid_to_username() {
        let m = UserAttributeMapper::keycloak_defaults();
        let u = m.map_entry(
            "uid=jdoe,ou=people,dc=example,dc=com",
            &entry(&[("uid", &["jdoe"])]),
        );
        assert_eq!(u.username, "jdoe");
    }

    #[test]
    fn maps_mail_to_email() {
        let m = UserAttributeMapper::keycloak_defaults();
        let u = m.map_entry(
            "uid=jdoe,ou=people,dc=example,dc=com",
            &entry(&[("mail", &["jdoe@example.com"])]),
        );
        assert_eq!(u.email.as_deref(), Some("jdoe@example.com"));
    }

    #[test]
    fn maps_given_name_and_sn_separately() {
        let m = UserAttributeMapper::keycloak_defaults();
        let u = m.map_entry(
            "uid=jdoe,ou=people,dc=example,dc=com",
            &entry(&[("givenName", &["Jane"]), ("sn", &["Doe"])]),
        );
        assert_eq!(u.first_name.as_deref(), Some("Jane"));
        assert_eq!(u.last_name.as_deref(), Some("Doe"));
    }

    #[test]
    fn maps_cn_to_display_name() {
        let m = UserAttributeMapper::keycloak_defaults();
        let u = m.map_entry(
            "uid=jdoe,ou=people,dc=example,dc=com",
            &entry(&[("cn", &["Jane Doe"])]),
        );
        assert_eq!(u.display_name.as_deref(), Some("Jane Doe"));
    }

    #[test]
    fn maps_member_of_to_group_cn_list() {
        let m = UserAttributeMapper::keycloak_defaults();
        let u = m.map_entry(
            "uid=jdoe,ou=people,dc=example,dc=com",
            &entry(&[(
                "memberOf",
                &[
                    "cn=engineers,ou=groups,dc=example,dc=com",
                    "cn=on-call,ou=groups,dc=example,dc=com",
                ],
            )]),
        );
        assert_eq!(u.groups, vec!["engineers", "on-call"]);
    }

    #[test]
    fn attribute_lookup_is_case_insensitive_when_configured() {
        let m = UserAttributeMapper::keycloak_defaults();
        let u = m.map_entry(
            "dn",
            // 'UID' upper-cased — RFC 4517 caseIgnoreMatch
            &entry(&[("UID", &["jdoe"])]),
        );
        assert_eq!(u.username, "jdoe");
    }

    #[test]
    fn unmapped_attrs_carry_through_as_other_attrs() {
        let m = UserAttributeMapper::keycloak_defaults();
        let u = m.map_entry(
            "dn",
            &entry(&[("uid", &["jdoe"]), ("telephoneNumber", &["555-1234"])]),
        );
        assert_eq!(
            u.other_attrs.get("telephoneNumber").map(Vec::as_slice),
            Some(["555-1234".to_string()].as_slice())
        );
    }

    #[test]
    fn extract_cn_from_dn_handles_canonical_form() {
        assert_eq!(
            extract_cn_from_dn("cn=engineers,ou=groups,dc=example,dc=com")
                .as_deref(),
            Some("engineers")
        );
    }

    #[test]
    fn extract_cn_from_dn_returns_none_for_non_cn_head() {
        assert!(extract_cn_from_dn(
            "ou=groups,dc=example,dc=com"
        )
        .is_none());
    }

    #[test]
    fn extract_cn_from_dn_is_case_insensitive_on_head() {
        assert_eq!(
            extract_cn_from_dn("CN=engineers,ou=groups,dc=example,dc=com")
                .as_deref(),
            Some("engineers")
        );
    }
}

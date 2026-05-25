// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 federation/ldap/src/main/java/org/keycloak/storage/ldap/mappers/membership/group/GroupLDAPStorageMapper.java

//! LDAP group-membership sync. Keycloak's
//! `GroupLDAPStorageMapper` supports two strategies — which side
//! of the relationship holds the back-reference:
//!
//! 1. **`memberOf` (user-attribute)** — each user entry lists
//!    every group DN it belongs to. Modern AD + recent OpenLDAP
//!    `memberOf` overlay default.
//! 2. **`member` (group-attribute)** — each group entry lists
//!    every user DN that belongs to it. Classic OpenLDAP
//!    posixGroup / groupOfNames layout.
//!
//! Both models flatten down to the same `Vec<String>` of group
//! names exposed to cave-auth — the mapper choice only changes
//! which LDAP search the federation provider runs.

use std::collections::BTreeMap;

use super::user_mapper::extract_cn_from_dn;

/// Which side of the user↔group edge stores the link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupMembershipModel {
    /// `memberOf` lives on the user entry. Read each user's
    /// `memberOf` and project the group CNs.
    UserMemberOf,
    /// `member` lives on the group entry. Read each group's
    /// `member` list and invert.
    GroupMember,
}

/// Group sync configuration.
#[derive(Debug, Clone)]
pub struct GroupMapper {
    pub model: GroupMembershipModel,
    /// `memberOf` (in `UserMemberOf`) or `member` (in
    /// `GroupMember`) — LDAP attribute name.
    pub membership_attr: String,
    /// LDAP attribute holding the group's name (typically
    /// `cn`). Used to project a DN list down to display names.
    pub group_name_attr: String,
}

impl GroupMapper {
    /// Default for Active Directory + recent OpenLDAP.
    pub fn member_of_default() -> Self {
        GroupMapper {
            model: GroupMembershipModel::UserMemberOf,
            membership_attr: "memberOf".into(),
            group_name_attr: "cn".into(),
        }
    }
    /// Default for legacy `groupOfNames` / `posixGroup` schemas.
    pub fn group_member_default() -> Self {
        GroupMapper {
            model: GroupMembershipModel::GroupMember,
            membership_attr: "member".into(),
            group_name_attr: "cn".into(),
        }
    }

    /// Project a user entry's group membership down to a
    /// `Vec<String>` of group CNs. Used in
    /// [`GroupMembershipModel::UserMemberOf`] mode. Group DNs
    /// from `memberOf` are reduced to CN via
    /// [`extract_cn_from_dn`] — same trick Keycloak's
    /// `LDAPUtils.getMemberAttributeValue` does.
    pub fn groups_of_user(&self, user_entry: &BTreeMap<String, Vec<String>>) -> Vec<String> {
        let key = user_entry
            .keys()
            .find(|k| k.eq_ignore_ascii_case(&self.membership_attr));
        let Some(key) = key else { return Vec::new() };
        let Some(dns) = user_entry.get(key) else {
            return Vec::new();
        };
        dns.iter()
            .map(|dn| extract_cn_from_dn(dn).unwrap_or_else(|| dn.clone()))
            .collect()
    }

    /// Invert a group→members listing into a `user_dn → groups`
    /// map. Used in [`GroupMembershipModel::GroupMember`] mode.
    pub fn invert_group_members(&self, groups: &[GroupEntry]) -> BTreeMap<String, Vec<String>> {
        let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for g in groups {
            for member_dn in &g.members {
                out.entry(member_dn.to_owned())
                    .or_default()
                    .push(g.name.clone());
            }
        }
        out
    }
}

/// One LDAP group, as returned by `(objectClass=groupOfNames)`
/// or `(objectClass=group)` (AD) searches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupEntry {
    pub dn: String,
    pub name: String,
    pub members: Vec<String>,
}

impl GroupEntry {
    /// Parse a group entry's attribute map. Picks the configured
    /// `group_name_attr` for the human-readable name; falls back
    /// to the DN's CN if the attribute is absent.
    pub fn from_attrs(
        mapper: &GroupMapper,
        dn: &str,
        attrs: &BTreeMap<String, Vec<String>>,
    ) -> Self {
        let name_key = attrs
            .keys()
            .find(|k| k.eq_ignore_ascii_case(&mapper.group_name_attr));
        let name = name_key
            .and_then(|k| attrs.get(k))
            .and_then(|v| v.first())
            .cloned()
            .unwrap_or_else(|| extract_cn_from_dn(dn).unwrap_or_else(|| dn.to_owned()));
        let member_key = attrs
            .keys()
            .find(|k| k.eq_ignore_ascii_case(&mapper.membership_attr));
        let members = member_key
            .and_then(|k| attrs.get(k).cloned())
            .unwrap_or_default();
        GroupEntry {
            dn: dn.to_owned(),
            name,
            members,
        }
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
    fn member_of_mode_projects_group_dns_to_cns() {
        let m = GroupMapper::member_of_default();
        let groups = m.groups_of_user(&entry(&[(
            "memberOf",
            &[
                "cn=engineers,ou=groups,dc=example,dc=com",
                "cn=on-call,ou=groups,dc=example,dc=com",
            ],
        )]));
        assert_eq!(groups, vec!["engineers", "on-call"]);
    }

    #[test]
    fn member_of_mode_returns_empty_when_attribute_absent() {
        let m = GroupMapper::member_of_default();
        let groups = m.groups_of_user(&entry(&[("uid", &["jdoe"])]));
        assert!(groups.is_empty());
    }

    #[test]
    fn group_member_mode_inverts_into_user_to_group_map() {
        let m = GroupMapper::group_member_default();
        let groups = vec![
            GroupEntry {
                dn: "cn=engineers,ou=groups".into(),
                name: "engineers".into(),
                members: vec!["uid=jdoe,ou=people".into(), "uid=asmith,ou=people".into()],
            },
            GroupEntry {
                dn: "cn=on-call,ou=groups".into(),
                name: "on-call".into(),
                members: vec!["uid=jdoe,ou=people".into()],
            },
        ];
        let inv = m.invert_group_members(&groups);
        assert_eq!(
            inv.get("uid=jdoe,ou=people"),
            Some(&vec!["engineers".to_string(), "on-call".to_string()])
        );
        assert_eq!(
            inv.get("uid=asmith,ou=people"),
            Some(&vec!["engineers".to_string()])
        );
    }

    #[test]
    fn group_entry_from_attrs_pulls_cn_and_members() {
        let m = GroupMapper::group_member_default();
        let g = GroupEntry::from_attrs(
            &m,
            "cn=engineers,ou=groups,dc=example,dc=com",
            &entry(&[
                ("cn", &["engineers"]),
                ("member", &["uid=a,ou=people", "uid=b,ou=people"]),
            ]),
        );
        assert_eq!(g.name, "engineers");
        assert_eq!(g.members.len(), 2);
    }

    #[test]
    fn group_entry_falls_back_to_dn_cn_when_name_attr_missing() {
        let m = GroupMapper::group_member_default();
        let g = GroupEntry::from_attrs(
            &m,
            "cn=on-call,ou=groups,dc=example,dc=com",
            &entry(&[("member", &["uid=a,ou=people"])]),
        );
        assert_eq!(g.name, "on-call");
    }

    #[test]
    fn member_of_attr_lookup_is_case_insensitive() {
        let m = GroupMapper::member_of_default();
        let groups = m.groups_of_user(&entry(&[("MEMBEROF", &["cn=engineers,ou=groups"])]));
        assert_eq!(groups, vec!["engineers"]);
    }
}

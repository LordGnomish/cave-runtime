// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@b825ba97 federation/ldap/src/main/java/org/keycloak/storage/ldap/mappers/LDAPStorageMapper.java
// Source: keycloak/keycloak@b825ba97 federation/ldap/src/main/java/org/keycloak/storage/ldap/mappers/membership/group/GroupLDAPStorageMapper.java
// Source: keycloak/keycloak@b825ba97 federation/ldap/src/main/java/org/keycloak/storage/ldap/mappers/UserAttributeLDAPStorageMapper.java
// Source: keycloak/keycloak@b825ba97 federation/ldap/src/main/java/org/keycloak/storage/ldap/mappers/membership/role/RoleLDAPStorageMapper.java
//
// LDAP -> Cave user attribute / group / role mappers.  Each mapper
// is a pure function from `LdapObject` + sink config -> changes on a
// `UserRecord`.

use super::object::LdapObject;
use super::sync::UserRecord;

/// Membership style — verbatim from `LDAPGroupMapperConfig`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MembershipStyle {
    /// AD-style: group object lists user DNs in `member`.
    DnReference,
    /// OpenLDAP / posixGroup: group object lists `memberUid`s
    /// (login names rather than DNs).
    UidReference,
}

/// User-attribute mapper.  Copies one LDAP attr into the cave user
/// model.  In Keycloak, `UserAttributeLDAPStorageMapper` writes both
/// directions; we currently only read from LDAP into cave (matches
/// `EditMode::ReadOnly`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserAttributeMapper {
    pub ldap_attr: String,
    pub cave_attr: String,
    pub mandatory: bool,
}

impl UserAttributeMapper {
    /// Apply onto an existing record.  Returns `true` if the value
    /// changed.  If `mandatory` and the value is missing, returns
    /// `Err`.
    pub fn apply(&self, src: &LdapObject, dst: &mut UserRecord) -> Result<bool, MappingError> {
        let val = src.first_str(&self.ldap_attr).map(String::from);
        match (val, self.mandatory) {
            (None, true) => Err(MappingError::MissingMandatory(self.ldap_attr.clone())),
            (None, false) => Ok(false),
            (Some(v), _) => {
                let changed = match self.cave_attr.as_str() {
                    "email" => {
                        let prev = dst.email.clone();
                        dst.email = Some(v);
                        prev != dst.email
                    }
                    "displayName" => {
                        let prev = dst.display_name.clone();
                        dst.display_name = Some(v);
                        prev != dst.display_name
                    }
                    _ => false,
                };
                Ok(changed)
            }
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MappingError {
    #[error("mandatory LDAP attribute `{0}` missing on entry")]
    MissingMandatory(String),
    #[error("bad DN syntax in `{0}`")]
    BadDn(String),
}

/// Group / role mapper.  Walks group objects + assigns
/// memberships into the user record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupMapper {
    pub groups_dn: String,
    pub membership_style: MembershipStyle,
    /// Attribute on the group object holding members.  `member` for
    /// AD, `memberUid` for posixGroup.
    pub membership_attr: String,
    /// Attribute on the group object holding the display name.
    /// `cn` everywhere.
    pub group_name_attr: String,
    /// Optional preserve-inheritance flag — when true, groups whose
    /// `memberOf` attribute lists another group propagate names.
    pub preserve_inheritance: bool,
}

impl GroupMapper {
    /// Walk a list of LDAP group entries and return the names the
    /// user belongs to.  `user_dn` is the LDAP DN; `user_uid` is
    /// the username — used for posixGroup-style lookups.
    pub fn user_groups(&self, groups: &[LdapObject], user_dn: &str, user_uid: &str) -> Vec<String> {
        let mut out = Vec::new();
        for g in groups {
            let Some(members) = g.get(&self.membership_attr) else {
                continue;
            };
            let me_b = match self.membership_style {
                MembershipStyle::DnReference => user_dn.as_bytes(),
                MembershipStyle::UidReference => user_uid.as_bytes(),
            };
            let is_member = members.values.iter().any(|v| v.eq_ignore_ascii_case_eq(me_b));
            if is_member {
                if let Some(name) = g.first_str(&self.group_name_attr) {
                    out.push(name.to_string());
                }
            }
        }
        if self.preserve_inheritance {
            propagate_inheritance(&mut out, groups, &self.membership_attr, &self.group_name_attr);
        }
        out.sort();
        out.dedup();
        out
    }
}

// Tiny helper so we don't compare DN bytes case-sensitively.
trait EqIgnoreAsciiCase {
    fn eq_ignore_ascii_case_eq(&self, other: &[u8]) -> bool;
}

impl EqIgnoreAsciiCase for Vec<u8> {
    fn eq_ignore_ascii_case_eq(&self, other: &[u8]) -> bool {
        self.len() == other.len()
            && self.iter().zip(other.iter()).all(|(a, b)| a.eq_ignore_ascii_case(b))
    }
}

fn propagate_inheritance(out: &mut Vec<String>, groups: &[LdapObject], membership_attr: &str, group_name_attr: &str) {
    // One level of transitive expansion — Keycloak's
    // `extractAllGroupNames` walks until fixed-point, but a single
    // pass is sufficient for the typical 2-tier hierarchy and
    // keeps state-space testable.
    let mut added = true;
    while added {
        added = false;
        let snapshot: Vec<String> = out.clone();
        for g in groups {
            let name = g.first_str(group_name_attr).unwrap_or("").to_string();
            if !snapshot.contains(&name) {
                continue;
            }
            if let Some(members) = g.get(membership_attr) {
                for parent_dn in &members.values {
                    if let Ok(s) = std::str::from_utf8(parent_dn) {
                        // For each parent group DN listed, find its cn.
                        if let Some(parent_cn) = groups.iter().find_map(|p| {
                            if p.dn.eq_ignore_ascii_case(s) {
                                p.first_str(group_name_attr).map(String::from)
                            } else {
                                None
                            }
                        }) {
                            if !out.contains(&parent_cn) {
                                out.push(parent_cn);
                                added = true;
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Role mapper — Keycloak's `RoleLDAPStorageMapper` is structurally
/// identical to `GroupLDAPStorageMapper`; we model it as a thin
/// wrapper that emits role names instead of group names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleMapper {
    pub inner: GroupMapper,
}

impl RoleMapper {
    pub fn user_roles(&self, groups: &[LdapObject], user_dn: &str, user_uid: &str) -> Vec<String> {
        self.inner.user_groups(groups, user_dn, user_uid)
    }
}

/// Build a stock OpenLDAP user-attribute mapper set.
pub fn openldap_default_attribute_mappers() -> Vec<UserAttributeMapper> {
    vec![
        UserAttributeMapper { ldap_attr: "mail".into(), cave_attr: "email".into(), mandatory: false },
        UserAttributeMapper { ldap_attr: "cn".into(), cave_attr: "displayName".into(), mandatory: false },
    ]
}

/// Build a stock AD user-attribute mapper set.
pub fn ad_default_attribute_mappers() -> Vec<UserAttributeMapper> {
    vec![
        UserAttributeMapper { ldap_attr: "mail".into(), cave_attr: "email".into(), mandatory: false },
        UserAttributeMapper { ldap_attr: "displayName".into(), cave_attr: "displayName".into(), mandatory: false },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(uid: &str) -> UserRecord {
        UserRecord {
            external_id: format!("uuid-{uid}"),
            username: uid.into(),
            email: None,
            display_name: None,
            roles: Vec::new(),
            groups: Vec::new(),
        }
    }

    fn ldap_user_alice() -> LdapObject {
        let mut o = LdapObject::new("uid=alice,dc=acme");
        o.set("uid", "alice");
        o.set("mail", "alice@acme.corp");
        o.set("cn", "Alice Adminerson");
        o
    }

    #[test]
    fn user_attribute_mapper_writes_email() {
        let m = UserAttributeMapper { ldap_attr: "mail".into(), cave_attr: "email".into(), mandatory: false };
        let mut rec = user("alice");
        assert!(m.apply(&ldap_user_alice(), &mut rec).unwrap());
        assert_eq!(rec.email.as_deref(), Some("alice@acme.corp"));
    }

    #[test]
    fn user_attribute_mapper_skips_missing_optional() {
        let m = UserAttributeMapper { ldap_attr: "telephoneNumber".into(), cave_attr: "phone".into(), mandatory: false };
        let mut rec = user("alice");
        assert_eq!(m.apply(&ldap_user_alice(), &mut rec).unwrap(), false);
    }

    #[test]
    fn user_attribute_mapper_errors_on_missing_mandatory() {
        let m = UserAttributeMapper { ldap_attr: "telephoneNumber".into(), cave_attr: "phone".into(), mandatory: true };
        let mut rec = user("alice");
        assert!(matches!(m.apply(&ldap_user_alice(), &mut rec), Err(MappingError::MissingMandatory(_))));
    }

    fn dn_group(name: &str, members: &[&str]) -> LdapObject {
        let mut o = LdapObject::new(format!("cn={name},ou=Groups,dc=acme"));
        o.set("cn", name);
        for m in members {
            o.set("member", *m);
        }
        o
    }

    #[test]
    fn group_mapper_dn_reference_matches_membership() {
        let mapper = GroupMapper {
            groups_dn: "ou=Groups,dc=acme".into(),
            membership_style: MembershipStyle::DnReference,
            membership_attr: "member".into(),
            group_name_attr: "cn".into(),
            preserve_inheritance: false,
        };
        let groups = vec![
            dn_group("admins", &["uid=alice,dc=acme", "uid=bob,dc=acme"]),
            dn_group("devs", &["uid=bob,dc=acme"]),
        ];
        let alice_groups = mapper.user_groups(&groups, "uid=alice,dc=acme", "alice");
        assert_eq!(alice_groups, vec!["admins"]);
        let bob_groups = mapper.user_groups(&groups, "uid=bob,dc=acme", "bob");
        assert_eq!(bob_groups, vec!["admins", "devs"]);
    }

    #[test]
    fn group_mapper_uid_reference_for_posixgroup() {
        let mapper = GroupMapper {
            groups_dn: "ou=Groups".into(),
            membership_style: MembershipStyle::UidReference,
            membership_attr: "memberUid".into(),
            group_name_attr: "cn".into(),
            preserve_inheritance: false,
        };
        let mut g = LdapObject::new("cn=staff,dc=acme");
        g.set("cn", "staff");
        g.set("memberUid", "alice");
        let groups = vec![g];
        let r = mapper.user_groups(&groups, "uid=alice,dc=acme", "alice");
        assert_eq!(r, vec!["staff"]);
    }

    #[test]
    fn role_mapper_delegates_to_group_mapper() {
        let inner = GroupMapper {
            groups_dn: "ou=Roles,dc=acme".into(),
            membership_style: MembershipStyle::DnReference,
            membership_attr: "member".into(),
            group_name_attr: "cn".into(),
            preserve_inheritance: false,
        };
        let mapper = RoleMapper { inner };
        let groups = vec![dn_group("ops", &["uid=alice,dc=acme"])];
        let r = mapper.user_roles(&groups, "uid=alice,dc=acme", "alice");
        assert_eq!(r, vec!["ops"]);
    }

    #[test]
    fn dn_match_is_case_insensitive() {
        let mapper = GroupMapper {
            groups_dn: "ou=Groups".into(),
            membership_style: MembershipStyle::DnReference,
            membership_attr: "member".into(),
            group_name_attr: "cn".into(),
            preserve_inheritance: false,
        };
        let g = dn_group("ops", &["UID=Alice,DC=acme"]);
        let groups = vec![g];
        // user_dn in lowercase still matches.
        let r = mapper.user_groups(&groups, "uid=alice,dc=acme", "alice");
        assert_eq!(r, vec!["ops"]);
    }

    #[test]
    fn default_mapper_sets_present() {
        assert!(!openldap_default_attribute_mappers().is_empty());
        assert!(!ad_default_attribute_mappers().is_empty());
    }
}

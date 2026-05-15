// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@b825ba97 federation/ldap/src/main/java/org/keycloak/storage/ldap/idm/model/LDAPObject.java
//
// In-memory representation of a single LDAP entry.  Mirrors
// `LDAPObject` — a DN, a UUID, and an attribute multimap.

use std::collections::BTreeMap;

/// One attribute on an LDAP entry.  Values are bytes because AD's
/// `objectGUID` etc. are binary.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LdapAttribute {
    pub name: String,
    pub values: Vec<Vec<u8>>,
}

impl LdapAttribute {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), values: Vec::new() }
    }

    pub fn single<V: Into<Vec<u8>>>(name: impl Into<String>, v: V) -> Self {
        Self { name: name.into(), values: vec![v.into()] }
    }

    pub fn push<V: Into<Vec<u8>>>(&mut self, v: V) -> &mut Self {
        self.values.push(v.into());
        self
    }

    /// First value as UTF-8 if possible.
    pub fn as_str(&self) -> Option<&str> {
        self.values.first().and_then(|v| std::str::from_utf8(v).ok())
    }
}

/// One LDAP entry — DN plus an ordered attribute map.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LdapObject {
    pub dn: String,
    pub object_classes: Vec<String>,
    pub attributes: BTreeMap<String, LdapAttribute>,
}

impl LdapObject {
    pub fn new(dn: impl Into<String>) -> Self {
        Self {
            dn: dn.into(),
            object_classes: Vec::new(),
            attributes: BTreeMap::new(),
        }
    }

    pub fn with_attr(mut self, attr: LdapAttribute) -> Self {
        self.attributes.insert(attr.name.clone(), attr);
        self
    }

    pub fn set<V: Into<Vec<u8>>>(&mut self, name: &str, value: V) {
        self.attributes
            .entry(name.to_string())
            .or_insert_with(|| LdapAttribute::new(name))
            .push(value);
    }

    pub fn get(&self, name: &str) -> Option<&LdapAttribute> {
        self.attributes.get(name)
    }

    /// Convenience: first-value-as-utf8 for the named attribute.
    pub fn first_str(&self, name: &str) -> Option<&str> {
        self.get(name).and_then(|a| a.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ldap_attribute_single_holds_one_value() {
        let a = LdapAttribute::single("cn", "alice");
        assert_eq!(a.values.len(), 1);
        assert_eq!(a.as_str(), Some("alice"));
    }

    #[test]
    fn ldap_attribute_push_grows() {
        let mut a = LdapAttribute::new("memberOf");
        a.push("cn=admins,dc=acme").push("cn=devs,dc=acme");
        assert_eq!(a.values.len(), 2);
    }

    #[test]
    fn ldap_object_set_creates_then_appends() {
        let mut o = LdapObject::new("uid=alice,dc=acme,dc=corp");
        o.set("mail", "alice@acme.corp");
        o.set("mail", "alice@example.org");
        let m = o.get("mail").unwrap();
        assert_eq!(m.values.len(), 2);
    }

    #[test]
    fn first_str_returns_none_for_binary() {
        let mut o = LdapObject::new("cn=x");
        o.set("objectGUID", vec![0xff, 0xfe, 0xfd]);
        assert!(o.first_str("objectGUID").is_none());
    }
}

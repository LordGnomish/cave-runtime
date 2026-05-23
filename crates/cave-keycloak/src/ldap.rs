// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! LDAP / AD user federation.
//!
//! Upstream: `federation/ldap/src/main/java/org/keycloak/storage/ldap/LDAPStorageProvider.java`
//! + `federation/ldap/src/main/java/org/keycloak/storage/ldap/idm/store/IdentityStore.java`.
//!
//! The cave MVP defines the trait surface + an in-process `InMemoryLdap`
//! that other modules can configure as a `LdapBackend` for tests. The
//! wire-level OpenLDAP / AD adapter lands as `cave-keycloak-ldap-net`
//! Phase 2 (see manifest [[scope_cuts]] ldap-network-runtime).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Mutex;

use crate::error::{KeycloakError, Result};

/// One configured LDAP federation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LdapConfig {
    pub alias: String,
    pub connection_url: String,
    pub bind_dn: String,
    pub bind_credential_keychain_handle: String, // `keychain:…`, never plaintext
    pub users_dn: String,
    pub username_attribute: String,
    pub rdn_attribute: String,
    pub uuid_attribute: String,
    pub user_object_classes: Vec<String>,
    pub start_tls: bool,
    pub use_truststore: bool,
}

impl LdapConfig {
    pub fn validate(&self) -> Result<()> {
        if !self.bind_credential_keychain_handle.starts_with("keychain:") {
            return Err(KeycloakError::LdapError(
                "bind_credential must reference a keychain handle (`keychain:…`), never inline".into(),
            ));
        }
        if self.alias.is_empty() {
            return Err(KeycloakError::LdapError("alias empty".into()));
        }
        if self.users_dn.is_empty() {
            return Err(KeycloakError::LdapError("users_dn empty".into()));
        }
        Ok(())
    }
}

/// LDAP record as we see it after parsing the BER response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LdapEntry {
    pub dn: String,
    pub uuid: String,
    pub attributes: BTreeMap<String, Vec<String>>,
}

impl LdapEntry {
    pub fn first(&self, k: &str) -> Option<&str> {
        self.attributes.get(k).and_then(|v| v.first().map(|s| s.as_str()))
    }
}

/// Pluggable backend so tests don't need a real LDAP server.
pub trait LdapBackend: Send + Sync {
    fn bind(&self, dn: &str, password: &str) -> Result<()>;
    fn search(&self, base_dn: &str, filter: &str) -> Result<Vec<LdapEntry>>;
}

/// In-memory backend — purely for tests and the cave-keycloak local-dev
/// realm "ldap-mock".
pub struct InMemoryLdap {
    inner: Mutex<InMemoryInner>,
}

struct InMemoryInner {
    creds: BTreeMap<String, String>,
    entries: Vec<LdapEntry>,
}

impl Default for InMemoryLdap {
    fn default() -> Self {
        Self {
            inner: Mutex::new(InMemoryInner {
                creds: BTreeMap::new(),
                entries: Vec::new(),
            }),
        }
    }
}

impl InMemoryLdap {
    pub fn insert(&self, dn: &str, password: &str, entry: LdapEntry) {
        let mut g = self.inner.lock().unwrap();
        g.creds.insert(dn.to_string(), password.to_string());
        g.entries.push(entry);
    }
}

impl LdapBackend for InMemoryLdap {
    fn bind(&self, dn: &str, password: &str) -> Result<()> {
        let g = self.inner.lock().unwrap();
        let ok = g.creds.get(dn).map(|p| p == password).unwrap_or(false);
        if ok { Ok(()) } else { Err(KeycloakError::LdapError("invalid bind credentials".into())) }
    }

    fn search(&self, base_dn: &str, filter: &str) -> Result<Vec<LdapEntry>> {
        let g = self.inner.lock().unwrap();
        let f = parse_simple_filter(filter)?;
        let out: Vec<_> = g
            .entries
            .iter()
            .filter(|e| e.dn.ends_with(base_dn))
            .filter(|e| f.matches(e))
            .cloned()
            .collect();
        Ok(out)
    }
}

/// Minimal LDAP filter — supports `(attr=value)`, `(attr=*)`, and `(&...)`
/// conjunctions of the above. Enough for the username + objectClass
/// filters Keycloak emits during a sync; anything else returns an error.
struct SimpleFilter {
    parts: Vec<(String, String)>,
}

impl SimpleFilter {
    fn matches(&self, entry: &LdapEntry) -> bool {
        self.parts.iter().all(|(k, v)| {
            entry
                .attributes
                .get(k)
                .map(|vals| {
                    if v == "*" {
                        !vals.is_empty()
                    } else {
                        vals.iter().any(|av| av == v)
                    }
                })
                .unwrap_or(false)
        })
    }
}

fn parse_simple_filter(filter: &str) -> Result<SimpleFilter> {
    let s = filter.trim();
    if let Some(inner) = s.strip_prefix("(&").and_then(|s| s.strip_suffix(')')) {
        let mut parts = Vec::new();
        let mut depth = 0;
        let mut start = 0;
        let bytes = inner.as_bytes();
        for (i, &b) in bytes.iter().enumerate() {
            match b {
                b'(' => {
                    if depth == 0 { start = i; }
                    depth += 1;
                }
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        let one = &inner[start..=i];
                        let p = parse_atom(one)?;
                        parts.push(p);
                    }
                }
                _ => {}
            }
        }
        return Ok(SimpleFilter { parts });
    }
    Ok(SimpleFilter { parts: vec![parse_atom(s)?] })
}

fn parse_atom(s: &str) -> Result<(String, String)> {
    let s = s.trim();
    let inner = s
        .strip_prefix('(')
        .and_then(|x| x.strip_suffix(')'))
        .ok_or_else(|| KeycloakError::LdapError(format!("filter atom: {}", s)))?;
    let (k, v) = inner
        .split_once('=')
        .ok_or_else(|| KeycloakError::LdapError(format!("filter '=' missing: {}", inner)))?;
    Ok((k.to_string(), v.to_string()))
}

/// Authenticate a Keycloak user against the federation. Returns the
/// matched `LdapEntry`; the caller maps attributes → `User`.
pub fn authenticate(backend: &dyn LdapBackend, cfg: &LdapConfig, username: &str, password: &str) -> Result<LdapEntry> {
    cfg.validate()?;
    let filter = format!("(&({}={})(objectClass={}))", cfg.username_attribute, username, cfg.user_object_classes.first().cloned().unwrap_or_else(|| "person".into()));
    let entries = backend.search(&cfg.users_dn, &filter)?;
    let entry = entries
        .into_iter()
        .next()
        .ok_or_else(|| KeycloakError::LdapError(format!("no entry for {}", username)))?;
    backend.bind(&entry.dn, password)?;
    Ok(entry)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> LdapConfig {
        LdapConfig {
            alias: "corp".into(),
            connection_url: "ldap://corp.cave:389".into(),
            bind_dn: "cn=svc,ou=Services,dc=cave".into(),
            bind_credential_keychain_handle: "keychain:cave-keycloak/ldap/corp".into(),
            users_dn: "ou=People,dc=cave".into(),
            username_attribute: "uid".into(),
            rdn_attribute: "uid".into(),
            uuid_attribute: "entryUUID".into(),
            user_object_classes: vec!["inetOrgPerson".into()],
            start_tls: true,
            use_truststore: true,
        }
    }

    fn alice() -> LdapEntry {
        let mut a = BTreeMap::new();
        a.insert("uid".into(), vec!["alice".into()]);
        a.insert("entryUUID".into(), vec!["uuid-1".into()]);
        a.insert("objectClass".into(), vec!["inetOrgPerson".into()]);
        a.insert("mail".into(), vec!["alice@cave".into()]);
        LdapEntry {
            dn: "uid=alice,ou=People,dc=cave".into(),
            uuid: "uuid-1".into(),
            attributes: a,
        }
    }

    #[test]
    fn config_rejects_plaintext_bind_credential() {
        let mut c = config();
        c.bind_credential_keychain_handle = "literal-password".into();
        assert!(c.validate().is_err());
    }

    #[test]
    fn in_memory_bind_succeeds_for_known_credentials() {
        let b = InMemoryLdap::default();
        b.insert("uid=alice,ou=People,dc=cave", "secret", alice());
        b.bind("uid=alice,ou=People,dc=cave", "secret").unwrap();
        assert!(b.bind("uid=alice,ou=People,dc=cave", "wrong").is_err());
    }

    #[test]
    fn search_filters_by_uid_and_object_class() {
        let b = InMemoryLdap::default();
        b.insert("uid=alice,ou=People,dc=cave", "secret", alice());
        let results = b
            .search("ou=People,dc=cave", "(&(uid=alice)(objectClass=inetOrgPerson))")
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].first("mail"), Some("alice@cave"));
    }

    #[test]
    fn search_skips_entries_outside_base() {
        let b = InMemoryLdap::default();
        let mut entry = alice();
        entry.dn = "uid=bob,ou=Robots,dc=cave".into();
        b.insert(&entry.dn.clone(), "secret", entry);
        let results = b
            .search("ou=People,dc=cave", "(uid=bob)")
            .unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn authenticate_happy_path() {
        let b = InMemoryLdap::default();
        b.insert("uid=alice,ou=People,dc=cave", "secret", alice());
        let cfg = config();
        let e = authenticate(&b, &cfg, "alice", "secret").unwrap();
        assert_eq!(e.first("uid"), Some("alice"));
    }

    #[test]
    fn authenticate_wrong_password_fails() {
        let b = InMemoryLdap::default();
        b.insert("uid=alice,ou=People,dc=cave", "secret", alice());
        assert!(authenticate(&b, &config(), "alice", "wrong").is_err());
    }

    #[test]
    fn authenticate_unknown_user_fails() {
        let b = InMemoryLdap::default();
        let cfg = config();
        assert!(authenticate(&b, &cfg, "ghost", "anything").is_err());
    }
}

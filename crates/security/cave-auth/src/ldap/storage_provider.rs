// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 federation/ldap/src/main/java/org/keycloak/storage/ldap/LDAPStorageProvider.java

//! `UserStorageProvider` equivalent. Keycloak's
//! `LDAPStorageProvider` is the federation surface — the bit of
//! Keycloak that adapts an LDAP directory to its `UserModel`
//! API. cave-auth's [`UserStorageProvider`] trait carries the
//! same shape: search-by-username / search-by-email /
//! list-with-filter / count.
//!
//! Two implementations ship in this crate:
//!
//! * [`LdapStorageProvider`] — the real federation provider,
//!   accepting a pluggable [`LdapTransport`] so cave's runtime
//!   can wire it to whatever socket layer it prefers (TLS,
//!   plain TCP, in-memory for tests).
//! * [`InMemoryDirectory`] — an in-memory directory for tests.
//!   It implements [`UserStorageProvider`] directly, evaluating
//!   filters in pure Rust via [`crate::ldap::query::Filter::matches`].
//!
//! ## Honest gap
//!
//! The trait does **not** expose a live `search` against a
//! remote LDAP server in the way Keycloak's
//! `LDAPIdentityStore.fetchQueryResults` does — encoding the
//! BER `SearchRequest` and parsing the `SearchResultEntry` /
//! `SearchResultDone` stream is significant volume that's
//! gated on a real-server test harness. The connection layer
//! ([`crate::ldap::connection`]) has the BindRequest/Response
//! path; SearchRequest BER is tracked as a manifest gap and
//! intentionally documented honestly.

use std::collections::BTreeMap;

use super::LdapError;
use super::group_mapper::GroupMapper;
use super::query::{Filter, LdapSearchSpec, Scope};
use super::user_mapper::{LdapUser, UserAttributeMapper};

/// What every cave-auth federation provider has to answer.
/// Synchronous trait — async wrapping is a job for the cave
/// runtime layer.
pub trait UserStorageProvider {
    /// Find one user by their cave-auth username (which is
    /// whatever the attribute mapper produced for the
    /// `username` field).
    fn find_by_username(&self, username: &str) -> Result<Option<LdapUser>, LdapError>;
    /// Find one user by their email.
    fn find_by_email(&self, email: &str) -> Result<Option<LdapUser>, LdapError>;
    /// Run a search spec, return every matching user.
    fn search(&self, spec: &LdapSearchSpec) -> Result<Vec<LdapUser>, LdapError>;
    /// Count users matching the search spec — used by the admin
    /// page to render "N users in this federation provider".
    fn count(&self, spec: &LdapSearchSpec) -> Result<usize, LdapError>;
}

/// Pluggable transport — feed-in bytes-out abstraction. Real
/// production code wires this to a `rustls` TLS stream; tests
/// wire it to an in-memory buffer.
pub trait LdapTransport: Send + Sync {
    /// Send one outbound BER frame and read back the response.
    /// The response may be a single LDAPMessage (BindResponse,
    /// SearchResultDone) or a stream of them (SearchResultEntry
    /// terminated by SearchResultDone). Callers parse the bytes.
    fn round_trip(&mut self, request: &[u8]) -> Result<Vec<u8>, LdapError>;
}

/// Federation provider config — what Keycloak's admin UI calls
/// "LDAP user federation".
#[derive(Debug, Clone)]
pub struct LdapStorageConfig {
    /// Bind DN — the service account cave-auth authenticates as.
    pub bind_dn: String,
    /// Bind password.
    pub bind_password: String,
    /// Search base — `dc=example,dc=com` etc.
    pub user_search_base: String,
    /// Group search base — typically `ou=groups,dc=example,dc=com`.
    pub group_search_base: String,
    /// Attribute-mapping table.
    pub user_mapper: UserAttributeMapper,
    /// Group-sync model + attribute names.
    pub group_mapper: GroupMapper,
    /// Whether this provider is "AD"-flavoured (selects
    /// `active_directory` mapping rules).
    pub is_active_directory: bool,
}

impl LdapStorageConfig {
    /// Sensible defaults for an OpenLDAP-shaped directory.
    pub fn openldap_default(base_dn: impl Into<String>) -> Self {
        let base = base_dn.into();
        LdapStorageConfig {
            bind_dn: format!("cn=admin,{base}"),
            bind_password: String::new(),
            user_search_base: format!("ou=people,{base}"),
            group_search_base: format!("ou=groups,{base}"),
            user_mapper: UserAttributeMapper::keycloak_defaults(),
            group_mapper: GroupMapper::member_of_default(),
            is_active_directory: false,
        }
    }
    /// Active Directory shape — `sAMAccountName` instead of
    /// `uid`, `userPrincipalName` for email.
    pub fn active_directory_default(base_dn: impl Into<String>) -> Self {
        let base = base_dn.into();
        let mut user_mapper = UserAttributeMapper::keycloak_defaults();
        // Switch the username row from `uid` to `sAMAccountName`.
        if let Some(row) = user_mapper
            .rows
            .iter_mut()
            .find(|r| r.user_field == "username")
        {
            row.ldap_attr = "sAMAccountName".into();
        }
        if let Some(row) = user_mapper
            .rows
            .iter_mut()
            .find(|r| r.user_field == "email")
        {
            row.ldap_attr = "userPrincipalName".into();
        }
        LdapStorageConfig {
            bind_dn: format!("cn=cave-svc,cn=Users,{base}"),
            bind_password: String::new(),
            user_search_base: format!("cn=Users,{base}"),
            group_search_base: format!("cn=Users,{base}"),
            user_mapper,
            group_mapper: GroupMapper::member_of_default(),
            is_active_directory: true,
        }
    }
}

/// Real federation provider. Currently can drive the bind path
/// against a [`LdapTransport`]; live search uses the in-memory
/// shadow directory ([`InMemoryDirectory`]) the runtime layer
/// passes in.
pub struct LdapStorageProvider {
    pub config: LdapStorageConfig,
    pub directory: InMemoryDirectory,
}

impl LdapStorageProvider {
    pub fn new(config: LdapStorageConfig, directory: InMemoryDirectory) -> Self {
        LdapStorageProvider { config, directory }
    }

    /// Drive the bind handshake. Encodes a simple-bind frame and
    /// asks the transport for the response.
    pub fn bind<T: LdapTransport>(
        &self,
        transport: &mut T,
    ) -> Result<super::ResultCode, LdapError> {
        let conn = super::connection::LdapConnection::new();
        let frame = conn.encode_bind_request(
            &self.config.bind_dn,
            &super::connection::BindAuth::Simple(self.config.bind_password.clone()),
        );
        let response = transport.round_trip(&frame)?;
        let parsed = super::connection::BindResponse::parse(&response)?;
        if !parsed.result_code.is_success() {
            return Err(LdapError::BindFailed(parsed.diagnostic_message));
        }
        Ok(parsed.result_code)
    }
}

impl UserStorageProvider for LdapStorageProvider {
    fn find_by_username(&self, username: &str) -> Result<Option<LdapUser>, LdapError> {
        self.directory.find_by_username(username)
    }
    fn find_by_email(&self, email: &str) -> Result<Option<LdapUser>, LdapError> {
        self.directory.find_by_email(email)
    }
    fn search(&self, spec: &LdapSearchSpec) -> Result<Vec<LdapUser>, LdapError> {
        self.directory.search(spec)
    }
    fn count(&self, spec: &LdapSearchSpec) -> Result<usize, LdapError> {
        self.directory.count(spec)
    }
}

/// In-memory directory — what tests use, and what
/// [`LdapStorageProvider`] composes for offline mode.
#[derive(Debug, Clone, Default)]
pub struct InMemoryDirectory {
    /// DN → attribute map.
    pub entries: BTreeMap<String, BTreeMap<String, Vec<String>>>,
    pub mapper: UserAttributeMapper,
}

impl InMemoryDirectory {
    pub fn new() -> Self {
        Self {
            mapper: UserAttributeMapper::keycloak_defaults(),
            ..Default::default()
        }
    }
    pub fn insert(&mut self, dn: impl Into<String>, attrs: BTreeMap<String, Vec<String>>) {
        self.entries.insert(dn.into(), attrs);
    }

    fn entries_matching(
        &self,
        spec: &LdapSearchSpec,
    ) -> Vec<(&String, &BTreeMap<String, Vec<String>>)> {
        self.entries
            .iter()
            .filter(|(dn, _)| match spec.scope {
                Scope::Base => dn.as_str() == spec.base_dn.as_str(),
                Scope::OneLevel => {
                    // immediate child = exactly one ',' between
                    // child-rdn and base-dn.
                    if !dn.ends_with(&spec.base_dn) || dn.as_str() == spec.base_dn {
                        return false;
                    }
                    let head = &dn[..dn.len() - spec.base_dn.len()];
                    let head = head.trim_end_matches(',');
                    !head.contains(',')
                }
                Scope::Subtree => dn.ends_with(&spec.base_dn) || spec.base_dn.is_empty(),
            })
            .filter(|(_, attrs)| spec.filter.matches(attrs))
            .collect()
    }
}

impl UserStorageProvider for InMemoryDirectory {
    fn find_by_username(&self, username: &str) -> Result<Option<LdapUser>, LdapError> {
        let filter = Filter::Equal {
            attr: self
                .mapper
                .rows
                .iter()
                .find(|r| r.user_field == "username")
                .map(|r| r.ldap_attr.clone())
                .unwrap_or_else(|| "uid".into()),
            value: username.to_owned(),
        };
        for (dn, attrs) in &self.entries {
            if filter.matches(attrs) {
                return Ok(Some(self.mapper.map_entry(dn, attrs)));
            }
        }
        Ok(None)
    }
    fn find_by_email(&self, email: &str) -> Result<Option<LdapUser>, LdapError> {
        let attr = self
            .mapper
            .rows
            .iter()
            .find(|r| r.user_field == "email")
            .map(|r| r.ldap_attr.clone())
            .unwrap_or_else(|| "mail".into());
        let filter = Filter::Equal {
            attr,
            value: email.to_owned(),
        };
        for (dn, attrs) in &self.entries {
            if filter.matches(attrs) {
                return Ok(Some(self.mapper.map_entry(dn, attrs)));
            }
        }
        Ok(None)
    }
    fn search(&self, spec: &LdapSearchSpec) -> Result<Vec<LdapUser>, LdapError> {
        let matches = self.entries_matching(spec);
        let mut limit = spec.size_limit as usize;
        if limit == 0 {
            limit = usize::MAX;
        }
        Ok(matches
            .into_iter()
            .take(limit)
            .map(|(dn, attrs)| self.mapper.map_entry(dn, attrs))
            .collect())
    }
    fn count(&self, spec: &LdapSearchSpec) -> Result<usize, LdapError> {
        Ok(self.entries_matching(spec).len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ldap::ResultCode;
    use crate::ldap::query::{Filter, LdapQueryBuilder, Scope};

    fn seed_directory() -> InMemoryDirectory {
        let mut d = InMemoryDirectory::new();
        let jdoe: BTreeMap<String, Vec<String>> = [
            ("uid".to_string(), vec!["jdoe".to_string()]),
            ("mail".to_string(), vec!["jdoe@example.com".to_string()]),
            ("givenName".to_string(), vec!["Jane".to_string()]),
            ("sn".to_string(), vec!["Doe".to_string()]),
            ("cn".to_string(), vec!["Jane Doe".to_string()]),
            ("objectClass".to_string(), vec!["inetOrgPerson".to_string()]),
            (
                "memberOf".to_string(),
                vec!["cn=engineers,ou=groups,dc=example,dc=com".to_string()],
            ),
        ]
        .into_iter()
        .collect();
        let asmith: BTreeMap<String, Vec<String>> = [
            ("uid".to_string(), vec!["asmith".to_string()]),
            ("mail".to_string(), vec!["asmith@example.com".to_string()]),
            ("givenName".to_string(), vec!["Alice".to_string()]),
            ("sn".to_string(), vec!["Smith".to_string()]),
            ("cn".to_string(), vec!["Alice Smith".to_string()]),
            ("objectClass".to_string(), vec!["inetOrgPerson".to_string()]),
        ]
        .into_iter()
        .collect();
        d.insert("uid=jdoe,ou=people,dc=example,dc=com", jdoe);
        d.insert("uid=asmith,ou=people,dc=example,dc=com", asmith);
        d
    }

    #[test]
    fn find_by_username_returns_mapped_user() {
        let d = seed_directory();
        let u = d.find_by_username("jdoe").unwrap().unwrap();
        assert_eq!(u.username, "jdoe");
        assert_eq!(u.email.as_deref(), Some("jdoe@example.com"));
    }

    #[test]
    fn find_by_username_returns_none_when_absent() {
        let d = seed_directory();
        let u = d.find_by_username("ghost").unwrap();
        assert!(u.is_none());
    }

    #[test]
    fn find_by_email_returns_mapped_user() {
        let d = seed_directory();
        let u = d.find_by_email("asmith@example.com").unwrap().unwrap();
        assert_eq!(u.username, "asmith");
    }

    #[test]
    fn search_subtree_finds_all_under_base() {
        let d = seed_directory();
        let spec = LdapQueryBuilder::new("ou=people,dc=example,dc=com")
            .scope(Scope::Subtree)
            .add_filter(Filter::Present { attr: "uid".into() })
            .build();
        let users = d.search(&spec).unwrap();
        assert_eq!(users.len(), 2);
    }

    #[test]
    fn search_with_size_limit_truncates() {
        let d = seed_directory();
        let spec = LdapQueryBuilder::new("ou=people,dc=example,dc=com")
            .scope(Scope::Subtree)
            .add_filter(Filter::Present { attr: "uid".into() })
            .size_limit(1)
            .build();
        let users = d.search(&spec).unwrap();
        assert_eq!(users.len(), 1);
    }

    #[test]
    fn search_filter_intersects_with_scope() {
        let d = seed_directory();
        let spec = LdapQueryBuilder::new("ou=people,dc=example,dc=com")
            .scope(Scope::Subtree)
            .add_filter(Filter::Equal {
                attr: "uid".into(),
                value: "jdoe".into(),
            })
            .build();
        let users = d.search(&spec).unwrap();
        assert_eq!(users.len(), 1);
        assert_eq!(users[0].username, "jdoe");
    }

    #[test]
    fn count_matches_search_length() {
        let d = seed_directory();
        let spec = LdapQueryBuilder::new("ou=people,dc=example,dc=com")
            .scope(Scope::Subtree)
            .add_filter(Filter::Present { attr: "uid".into() })
            .build();
        assert_eq!(d.count(&spec).unwrap(), 2);
    }

    #[test]
    fn openldap_default_config_has_expected_base() {
        let c = LdapStorageConfig::openldap_default("dc=example,dc=com");
        assert_eq!(c.user_search_base, "ou=people,dc=example,dc=com");
        assert!(!c.is_active_directory);
    }

    #[test]
    fn ad_default_config_switches_username_to_samaccountname() {
        let c = LdapStorageConfig::active_directory_default("dc=example,dc=com");
        let row = c
            .user_mapper
            .rows
            .iter()
            .find(|r| r.user_field == "username")
            .unwrap();
        assert_eq!(row.ldap_attr, "sAMAccountName");
        assert!(c.is_active_directory);
    }

    // ── Transport-driven bind smoke ──────────────────────────────────────────

    struct MockTransport {
        /// Canned response handed out on every round_trip.
        response: Vec<u8>,
        /// Captured request — for assertion in tests.
        last_request: Vec<u8>,
    }

    impl LdapTransport for MockTransport {
        fn round_trip(&mut self, request: &[u8]) -> Result<Vec<u8>, LdapError> {
            self.last_request = request.to_vec();
            Ok(self.response.clone())
        }
    }

    #[test]
    fn bind_returns_success_when_server_echoes_success() {
        // Success BindResponse — messageID 1, resultCode 0.
        let bind_ok = vec![
            0x30, 0x0c, 0x02, 0x01, 0x01, 0x61, 0x07, 0x0a, 0x01, 0x00, 0x04, 0x00, 0x04, 0x00,
        ];
        let provider = LdapStorageProvider::new(
            LdapStorageConfig::openldap_default("dc=example,dc=com"),
            seed_directory(),
        );
        let mut transport = MockTransport {
            response: bind_ok,
            last_request: Vec::new(),
        };
        let rc = provider.bind(&mut transport).unwrap();
        assert_eq!(rc, ResultCode::Success);
        // request must carry the bind DN
        assert!(
            transport
                .last_request
                .windows(provider.config.bind_dn.len())
                .any(|w| w == provider.config.bind_dn.as_bytes())
        );
    }

    #[test]
    fn bind_returns_error_when_server_says_invalid_credentials() {
        let bind_bad = vec![
            0x30, 0x10, 0x02, 0x01, 0x02, 0x61, 0x0b, 0x0a, 0x01, 0x31, 0x04, 0x00, 0x04, 0x04,
            b'b', b'a', b'd', b'!',
        ];
        let provider = LdapStorageProvider::new(
            LdapStorageConfig::openldap_default("dc=example,dc=com"),
            seed_directory(),
        );
        let mut transport = MockTransport {
            response: bind_bad,
            last_request: Vec::new(),
        };
        let err = provider.bind(&mut transport).unwrap_err();
        assert!(matches!(err, LdapError::BindFailed(_)));
    }
}

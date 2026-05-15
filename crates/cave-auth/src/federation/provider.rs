// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@b825ba97 services/src/main/java/org/keycloak/storage/UserStorageProvider.java
// Source: keycloak/keycloak@b825ba97 federation/ldap/src/main/java/org/keycloak/storage/ldap/LDAPStorageProvider.java
//
// Federation backend trait — the abstraction `UserStorageProvider`
// fulfils in Keycloak.  Concrete backends (LDAP, Kerberos, future
// IPA/NIS) implement [`Federation`] and surface their configuration
// through [`FederationConfig`].
//
// Keycloak distinguishes:
//
// * `EditMode`        — `READ_ONLY` / `WRITABLE` / `UNSYNCED`
// * `SyncPolicy`      — `FULL`, `CHANGED_USERS_ONLY`, `IMPORT_USERS=false`
// * `Vendor`          — `AD`, `RHDS`, `TIVOLI`, `NOVELL`, `OPENLDAP`,
//                       `OTHER`.  Drives attribute defaults.
//
// We mirror those enums verbatim so the manifest map is honest.

use std::time::{Duration, SystemTime};

/// Distinguishes which kind of user store this provider talks to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FederationKind {
    /// LDAPv3-speaking directory (OpenLDAP, AD-LDS, 389-ds, etc.).
    Ldap,
    /// Pure Kerberos (no directory; just keytab-validated tickets).
    Kerberos,
}

/// Write-back policy.  Verbatim from Keycloak `EditMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditMode {
    /// LDAP writes are forbidden.
    ReadOnly,
    /// Writes propagate to LDAP.
    Writable,
    /// User edits are kept in cave-auth only; never written back.
    Unsynced,
}

impl EditMode {
    /// Parse the Keycloak string form.  Matches the upstream
    /// `LDAPConfig#getEditMode`.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "READ_ONLY" => Some(EditMode::ReadOnly),
            "WRITABLE" => Some(EditMode::Writable),
            "UNSYNCED" => Some(EditMode::Unsynced),
            _ => None,
        }
    }
}

/// Sync policy — drives the periodic
/// `UserStorageSyncTask` cadence.  Verbatim from `SyncPolicy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncPolicy {
    /// Re-imports every entry on every tick.
    FullSync,
    /// Only entries whose modify-time is newer than `last_sync`
    /// are processed.  Maps to AD `whenChanged` or OpenLDAP
    /// `modifyTimestamp`.
    ChangedOnly,
    /// No automatic sync — users are imported on first login only.
    OnDemand,
}

/// Vendor hint used to choose attribute defaults and AD-vs-OpenLDAP
/// quirks.  Verbatim from `LDAPConstants#VENDOR_*`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Vendor {
    Ad,
    Rhds,
    Tivoli,
    Novell,
    OpenLdap,
    Other,
}

impl Vendor {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "ad" => Some(Vendor::Ad),
            "rhds" => Some(Vendor::Rhds),
            "tivoli" => Some(Vendor::Tivoli),
            "edirectory" | "novell" => Some(Vendor::Novell),
            "other" => Some(Vendor::Other),
            // Keycloak uses both `OPENLDAP` and `openldap`
            "openldap" => Some(Vendor::OpenLdap),
            _ => None,
        }
    }

    /// Default `usernameLdapAttribute`.  AD uses `sAMAccountName`,
    /// everyone else uses `uid`.
    pub fn default_username_attr(self) -> &'static str {
        match self {
            Vendor::Ad => "sAMAccountName",
            _ => "uid",
        }
    }

    /// Default `rdnLdapAttribute`.  AD uses `cn`, OpenLDAP uses `uid`.
    pub fn default_rdn_attr(self) -> &'static str {
        match self {
            Vendor::Ad => "cn",
            _ => "uid",
        }
    }

    /// Default `uuidLdapAttribute`.  AD's `objectGUID` is binary; the
    /// rest are ASCII.
    pub fn default_uuid_attr(self) -> &'static str {
        match self {
            Vendor::Ad => "objectGUID",
            Vendor::Rhds => "nsuniqueid",
            Vendor::Novell => "guid",
            _ => "entryUUID",
        }
    }
}

/// Federation configuration — a single declarative struct that
/// captures all knobs the Keycloak Admin Console exposes under
/// `User Federation -> LDAP/Kerberos`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FederationConfig {
    pub id: String,
    pub display_name: String,
    pub kind: FederationKind,
    pub vendor: Vendor,
    pub edit_mode: EditMode,
    pub sync_policy: SyncPolicy,
    /// `ldap://host:389` or `ldaps://host:636`.
    pub connection_url: String,
    /// `cn=admin,dc=acme,dc=corp`.  Empty = anonymous bind.
    pub bind_dn: String,
    /// Plain bind password.  In production held in cave-vault.
    pub bind_credential: String,
    /// `dc=acme,dc=corp`.
    pub users_dn: String,
    pub username_attr: String,
    pub rdn_attr: String,
    pub uuid_attr: String,
    pub user_object_classes: Vec<String>,
    /// Optional LDAP filter (no parens) ANDed onto every search.
    pub custom_user_search_filter: Option<String>,
    pub connection_timeout: Duration,
    pub read_timeout: Duration,
    pub sync_period: Duration,
    /// Kerberos default realm — used by SPNEGO when no explicit realm
    /// is present in the AP-REQ.  `None` for pure LDAP providers.
    pub kerberos_realm: Option<String>,
    /// Service principal name (e.g. `HTTP/portal.acme.corp@ACME.CORP`).
    pub kerberos_server_principal: Option<String>,
    /// On-disk path to the keytab.  Parsed lazily at first SPNEGO
    /// challenge.
    pub kerberos_keytab_path: Option<String>,
}

impl FederationConfig {
    /// Build a typical OpenLDAP config.  Helper for tests + the
    /// Admin UI's "Test" button.
    pub fn openldap_template(id: &str, url: &str, users_dn: &str) -> Self {
        Self {
            id: id.to_string(),
            display_name: format!("OpenLDAP — {url}"),
            kind: FederationKind::Ldap,
            vendor: Vendor::OpenLdap,
            edit_mode: EditMode::ReadOnly,
            sync_policy: SyncPolicy::ChangedOnly,
            connection_url: url.to_string(),
            bind_dn: String::new(),
            bind_credential: String::new(),
            users_dn: users_dn.to_string(),
            username_attr: Vendor::OpenLdap.default_username_attr().into(),
            rdn_attr: Vendor::OpenLdap.default_rdn_attr().into(),
            uuid_attr: Vendor::OpenLdap.default_uuid_attr().into(),
            user_object_classes: vec!["inetOrgPerson".into(), "organizationalPerson".into()],
            custom_user_search_filter: None,
            connection_timeout: Duration::from_secs(5),
            read_timeout: Duration::from_secs(10),
            sync_period: Duration::from_secs(3600),
            kerberos_realm: None,
            kerberos_server_principal: None,
            kerberos_keytab_path: None,
        }
    }

    /// Build a typical Active Directory config.
    pub fn ad_template(id: &str, url: &str, users_dn: &str, realm: &str) -> Self {
        Self {
            id: id.to_string(),
            display_name: format!("Active Directory — {realm}"),
            kind: FederationKind::Ldap,
            vendor: Vendor::Ad,
            edit_mode: EditMode::ReadOnly,
            sync_policy: SyncPolicy::ChangedOnly,
            connection_url: url.to_string(),
            bind_dn: String::new(),
            bind_credential: String::new(),
            users_dn: users_dn.to_string(),
            username_attr: Vendor::Ad.default_username_attr().into(),
            rdn_attr: Vendor::Ad.default_rdn_attr().into(),
            uuid_attr: Vendor::Ad.default_uuid_attr().into(),
            user_object_classes: vec!["person".into(), "organizationalPerson".into(), "user".into()],
            custom_user_search_filter: None,
            connection_timeout: Duration::from_secs(5),
            read_timeout: Duration::from_secs(10),
            sync_period: Duration::from_secs(3600),
            kerberos_realm: Some(realm.to_string()),
            kerberos_server_principal: None,
            kerberos_keytab_path: None,
        }
    }
}

/// Statistics surfaced to the portal + Prometheus.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FederationStats {
    pub last_sync: Option<SystemTime>,
    pub users_imported: u64,
    pub bind_success: u64,
    pub bind_failure: u64,
    pub search_count: u64,
    pub spnego_negotiations: u64,
}

/// Federation backend errors — a 1:1 map to Keycloak `ModelException`
/// and `AuthenticationException` paths.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FederationError {
    #[error("LDAP protocol error: {0}")]
    Protocol(String),
    #[error("bind failed: invalid credentials")]
    InvalidCredentials,
    #[error("user `{0}` not found")]
    UserNotFound(String),
    #[error("attribute `{0}` missing on entry")]
    MissingAttribute(String),
    #[error("configuration invalid: {0}")]
    BadConfig(String),
    #[error("upstream returned RFC 4511 result code {0}")]
    LdapResult(u32),
    #[error("keytab error: {0}")]
    Keytab(String),
    #[error("SPNEGO error: {0}")]
    Spnego(String),
    /// Sentinel for the libgssapi-link gap honestly documented in
    /// the module doc.
    #[error("GSSAPI verification not linked in this build")]
    GssapiNotLinked,
}

/// The federation contract.  Synchronous; the LDAP client itself
/// is implemented as a state-machine over a `Read + Write` transport
/// so it remains both `std::io` and `tokio::io` friendly.
pub trait Federation: Send + Sync {
    fn config(&self) -> &FederationConfig;
    fn kind(&self) -> FederationKind {
        self.config().kind
    }
    fn stats(&self) -> FederationStats;

    /// Verify a username + password against the directory.  In
    /// Keycloak this is `LDAPStorageProviderFactory#validate`.
    fn authenticate(&self, username: &str, password: &str) -> Result<(), FederationError>;

    /// Import a user lazily on first login.  Returns the
    /// directory-shaped user record.
    fn import(&self, username: &str) -> Result<crate::federation::ldap::LdapObject, FederationError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vendor_parse_matches_keycloak_constants() {
        assert_eq!(Vendor::parse("ad"), Some(Vendor::Ad));
        assert_eq!(Vendor::parse("openldap"), Some(Vendor::OpenLdap));
        assert_eq!(Vendor::parse("OPENLDAP"), Some(Vendor::OpenLdap));
        assert_eq!(Vendor::parse("eDirectory"), Some(Vendor::Novell));
        assert_eq!(Vendor::parse(""), None);
    }

    #[test]
    fn vendor_defaults_track_keycloak_ldapconstants() {
        assert_eq!(Vendor::Ad.default_username_attr(), "sAMAccountName");
        assert_eq!(Vendor::OpenLdap.default_username_attr(), "uid");
        assert_eq!(Vendor::Ad.default_uuid_attr(), "objectGUID");
        assert_eq!(Vendor::OpenLdap.default_uuid_attr(), "entryUUID");
    }

    #[test]
    fn editmode_parse_matches_keycloak_enum() {
        assert_eq!(EditMode::parse("READ_ONLY"), Some(EditMode::ReadOnly));
        assert_eq!(EditMode::parse("WRITABLE"), Some(EditMode::Writable));
        assert_eq!(EditMode::parse("UNSYNCED"), Some(EditMode::Unsynced));
        assert!(EditMode::parse("garbage").is_none());
    }

    #[test]
    fn ad_template_sets_kerberos_realm() {
        let c = FederationConfig::ad_template("acme-ad", "ldap://dc.acme.corp", "DC=acme,DC=corp", "ACME.CORP");
        assert_eq!(c.kerberos_realm.as_deref(), Some("ACME.CORP"));
        assert_eq!(c.vendor, Vendor::Ad);
        assert_eq!(c.username_attr, "sAMAccountName");
    }

    #[test]
    fn openldap_template_has_no_kerberos() {
        let c = FederationConfig::openldap_template("acme-ol", "ldap://ldap.acme.corp", "dc=acme,dc=corp");
        assert!(c.kerberos_realm.is_none());
        assert_eq!(c.vendor, Vendor::OpenLdap);
        assert_eq!(c.uuid_attr, "entryUUID");
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@b825ba97 services/src/main/java/org/keycloak/services/managers/UserStorageSyncManager.java
// Source: keycloak/keycloak@b825ba97 federation/ldap/src/main/java/org/keycloak/storage/ldap/LDAPStorageProviderFactory.java
//
// User import + periodic sync state machine.  Pure logic — no
// network I/O — so the unit tests cover every transition.
//
// State diagram (mirrors Keycloak `UserStorageSyncTask`):
//
//   Idle
//     │ sync-now/timer
//     ▼
//   FetchingPage(cursor, fetched, modified_since)
//     │ page-result
//     ▼
//   ProcessingPage(page) ──┐
//     │                    │
//     │  more cookie?      │
//     ├── yes ──┐          │
//     │        ▼           │
//     │   FetchingPage     │
//     │                    │
//     └── no ─►Completed(stats)
//
// In `ChangedOnly` mode, the search filter is augmented with
// `(modifyTimestamp>=<last_sync>)` (RFC 4517 GeneralizedTime).

use std::collections::HashMap;
use std::time::SystemTime;

use super::object::LdapObject;
use super::search::PagedResultsState;
use crate::federation::provider::{SyncPolicy, Vendor};

/// Mode driving filter selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncMode {
    Full,
    Changed { last_sync: SystemTime },
    OnDemand,
}

impl From<(&SyncPolicy, Option<SystemTime>)> for SyncMode {
    fn from(value: (&SyncPolicy, Option<SystemTime>)) -> Self {
        match (value.0, value.1) {
            (SyncPolicy::FullSync, _) => SyncMode::Full,
            (SyncPolicy::ChangedOnly, Some(t)) => SyncMode::Changed { last_sync: t },
            (SyncPolicy::ChangedOnly, None) => SyncMode::Full,
            (SyncPolicy::OnDemand, _) => SyncMode::OnDemand,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncState {
    Idle,
    FetchingPage { fetched: usize, cursor: Option<PagedResultsState> },
    Completed(SyncStats),
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SyncStats {
    pub fetched: usize,
    pub created: usize,
    pub updated: usize,
    pub skipped: usize,
    pub errors: usize,
    pub modified_since: Option<SystemTime>,
}

/// In-memory user store the sync task writes through.  In production
/// this is `cave-auth::tokens` / cave-tenant.  Here we trait it so
/// tests can wire a `HashMap`-backed mock.
pub trait UserSink {
    /// Lookup by federated `uuid` (the LDAP-side immutable id).
    fn find_by_external_id(&self, ext_id: &str) -> Option<UserRecord>;
    fn upsert(&mut self, rec: UserRecord);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserRecord {
    pub external_id: String,
    pub username: String,
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub roles: Vec<String>,
    pub groups: Vec<String>,
}

impl UserRecord {
    /// Build from an LDAP entry given mapping rules.  Mirrors
    /// `LDAPStorageProvider#importUserFromLDAP`.
    pub fn from_ldap(obj: &LdapObject, vendor: Vendor, username_attr: &str, uuid_attr: &str) -> Self {
        let external_id = match vendor {
            Vendor::Ad => super::ad::object_guid_to_string(obj.get(uuid_attr).and_then(|a| a.values.first()).map(|v| v.as_slice()).unwrap_or_default()),
            _ => obj.first_str(uuid_attr).unwrap_or("").to_string(),
        };
        let username = obj.first_str(username_attr).unwrap_or("").to_string();
        let email = obj.first_str("mail").map(String::from);
        let display_name = obj.first_str("cn").or_else(|| obj.first_str("displayName")).map(String::from);
        UserRecord { external_id, username, email, display_name, roles: Vec::new(), groups: Vec::new() }
    }
}

#[derive(Default)]
pub struct InMemoryUserSink {
    by_id: HashMap<String, UserRecord>,
}

impl InMemoryUserSink {
    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }
}

impl UserSink for InMemoryUserSink {
    fn find_by_external_id(&self, ext_id: &str) -> Option<UserRecord> {
        self.by_id.get(ext_id).cloned()
    }
    fn upsert(&mut self, rec: UserRecord) {
        self.by_id.insert(rec.external_id.clone(), rec);
    }
}

/// Sync driver — accepts pages of LdapObjects + advances state.
pub struct SyncDriver {
    pub mode: SyncMode,
    pub state: SyncState,
    pub vendor: Vendor,
    pub username_attr: String,
    pub uuid_attr: String,
}

impl SyncDriver {
    pub fn new(mode: SyncMode, vendor: Vendor, username_attr: &str, uuid_attr: &str) -> Self {
        Self {
            mode,
            state: SyncState::Idle,
            vendor,
            username_attr: username_attr.to_string(),
            uuid_attr: uuid_attr.to_string(),
        }
    }

    /// Begin a sync run.
    pub fn start(&mut self, page_size: u32) {
        self.state = SyncState::FetchingPage {
            fetched: 0,
            cursor: Some(PagedResultsState { size: page_size, cookie: Vec::new() }),
        };
    }

    /// Consume one page; returns the updated stats.  If `done` is
    /// true the driver moves to `Completed`; else it stays in
    /// `FetchingPage` with the new cursor.
    pub fn ingest_page(&mut self, page: &[LdapObject], next_cookie: Option<Vec<u8>>, sink: &mut dyn UserSink) -> SyncStats {
        let mut stats = match &self.state {
            SyncState::FetchingPage { fetched, .. } => SyncStats { fetched: *fetched, ..Default::default() },
            _ => SyncStats::default(),
        };
        for obj in page {
            let rec = UserRecord::from_ldap(obj, self.vendor, &self.username_attr, &self.uuid_attr);
            if rec.username.is_empty() || rec.external_id.is_empty() {
                stats.skipped += 1;
                continue;
            }
            let prior = sink.find_by_external_id(&rec.external_id);
            sink.upsert(rec.clone());
            if prior.is_some() {
                stats.updated += 1;
            } else {
                stats.created += 1;
            }
            stats.fetched += 1;
        }
        match next_cookie {
            Some(c) if !c.is_empty() => {
                self.state = SyncState::FetchingPage {
                    fetched: stats.fetched,
                    cursor: Some(PagedResultsState { size: page_size_of(&self.state), cookie: c }),
                };
            }
            _ => {
                self.state = SyncState::Completed(stats.clone());
            }
        }
        stats
    }

    /// Mark the run as failed.
    pub fn fail(&mut self, reason: impl Into<String>) {
        self.state = SyncState::Failed(reason.into());
    }

    /// Build the modify-time filter shard `(modifyTimestamp>=…)` (OpenLDAP)
    /// or `(whenChanged>=…)` (AD).  Returns `None` for full sync.
    pub fn changed_since_filter_shard(&self) -> Option<String> {
        let last = match &self.mode {
            SyncMode::Changed { last_sync } => *last_sync,
            _ => return None,
        };
        let attr = match self.vendor {
            Vendor::Ad => "whenChanged",
            _ => "modifyTimestamp",
        };
        Some(format!("({attr}>={})", to_generalized_time(last)))
    }
}

fn page_size_of(state: &SyncState) -> u32 {
    match state {
        SyncState::FetchingPage { cursor: Some(p), .. } => p.size,
        _ => 100,
    }
}

/// Format a `SystemTime` as RFC 4517 GeneralizedTime
/// (`YYYYMMDDHHMMSSZ`).  AD accepts the same shape.
pub fn to_generalized_time(t: SystemTime) -> String {
    let dur = t.duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default();
    let secs = dur.as_secs() as i64;
    let (year, month, day, hour, minute, second) = epoch_to_ymdhms(secs);
    format!(
        "{:04}{:02}{:02}{:02}{:02}{:02}Z",
        year, month, day, hour, minute, second
    )
}

/// Naive epoch → calendar conversion (UTC, civil-from-days algorithm
/// after Howard Hinnant's `days_from_civil`).
fn epoch_to_ymdhms(secs: i64) -> (i64, u32, u32, u32, u32, u32) {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let hour = (rem / 3600) as u32;
    let minute = ((rem / 60) % 60) as u32;
    let second = (rem % 60) as u32;
    // civil_from_days, Hinnant 2013.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    (year, m, d, hour, minute, second)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn make_obj(uid: &str, mail: &str) -> LdapObject {
        let mut o = LdapObject::new(format!("uid={uid},dc=acme"));
        o.set("uid", uid);
        o.set("entryUUID", format!("uuid-{uid}"));
        o.set("cn", uid);
        o.set("mail", mail);
        o
    }

    #[test]
    fn from_policy_changedonly_with_no_last_sync_falls_back_to_full() {
        let m: SyncMode = (&SyncPolicy::ChangedOnly, None).into();
        assert!(matches!(m, SyncMode::Full));
    }

    #[test]
    fn from_policy_changedonly_with_last_sync_is_changed() {
        let now = SystemTime::now();
        let m: SyncMode = (&SyncPolicy::ChangedOnly, Some(now)).into();
        if let SyncMode::Changed { last_sync } = m {
            assert_eq!(last_sync, now);
        } else {
            panic!("expected Changed");
        }
    }

    #[test]
    fn driver_idle_to_fetching_to_completed() {
        let mut d = SyncDriver::new(SyncMode::Full, Vendor::OpenLdap, "uid", "entryUUID");
        let mut sink = InMemoryUserSink::default();
        assert_eq!(d.state, SyncState::Idle);
        d.start(100);
        assert!(matches!(d.state, SyncState::FetchingPage { .. }));
        let page = vec![make_obj("alice", "a@x"), make_obj("bob", "b@x")];
        let stats = d.ingest_page(&page, None, &mut sink);
        assert_eq!(stats.created, 2);
        assert_eq!(sink.len(), 2);
        assert!(matches!(d.state, SyncState::Completed(_)));
    }

    #[test]
    fn driver_multi_page_keeps_fetching() {
        let mut d = SyncDriver::new(SyncMode::Full, Vendor::OpenLdap, "uid", "entryUUID");
        let mut sink = InMemoryUserSink::default();
        d.start(2);
        let _ = d.ingest_page(&[make_obj("alice", "a@x")], Some(b"c1".to_vec()), &mut sink);
        assert!(matches!(d.state, SyncState::FetchingPage { .. }));
        let _ = d.ingest_page(&[make_obj("bob", "b@x")], None, &mut sink);
        assert!(matches!(d.state, SyncState::Completed(_)));
        assert_eq!(sink.len(), 2);
    }

    #[test]
    fn update_path_does_not_duplicate_user() {
        let mut d = SyncDriver::new(SyncMode::Full, Vendor::OpenLdap, "uid", "entryUUID");
        let mut sink = InMemoryUserSink::default();
        d.start(10);
        let _ = d.ingest_page(&[make_obj("alice", "a@x")], None, &mut sink);
        d.start(10);
        let stats = d.ingest_page(&[make_obj("alice", "a@example.org")], None, &mut sink);
        assert_eq!(stats.created, 0);
        assert_eq!(stats.updated, 1);
        assert_eq!(sink.len(), 1);
    }

    #[test]
    fn skips_objects_without_uid_or_uuid() {
        let mut d = SyncDriver::new(SyncMode::Full, Vendor::OpenLdap, "uid", "entryUUID");
        let mut sink = InMemoryUserSink::default();
        d.start(10);
        let bare = LdapObject::new("uid=,dc=acme");
        let stats = d.ingest_page(&[bare], None, &mut sink);
        assert_eq!(stats.skipped, 1);
        assert_eq!(sink.len(), 0);
    }

    #[test]
    fn changed_since_shard_uses_openldap_attr() {
        let then = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let d = SyncDriver::new(SyncMode::Changed { last_sync: then }, Vendor::OpenLdap, "uid", "entryUUID");
        let shard = d.changed_since_filter_shard().unwrap();
        assert!(shard.starts_with("(modifyTimestamp>="));
    }

    #[test]
    fn changed_since_shard_uses_ad_attr() {
        let then = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let d = SyncDriver::new(SyncMode::Changed { last_sync: then }, Vendor::Ad, "sAMAccountName", "objectGUID");
        let shard = d.changed_since_filter_shard().unwrap();
        assert!(shard.starts_with("(whenChanged>="));
    }

    #[test]
    fn generalized_time_format_matches_rfc_4517() {
        // 2023-11-14 22:13:20 UTC.
        let t = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        assert_eq!(to_generalized_time(t), "20231114221320Z");
    }
}

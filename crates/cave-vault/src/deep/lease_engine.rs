//! Lease lifecycle state machine.
//!
//! Mirrors the lease semantics in `vault/expiration.go`:
//!
//! * Each lease has a `created_at`, `ttl`, and `max_ttl`.
//! * Renewal extends TTL but never beyond `max_ttl`.
//! * Revocation cascades to children (parent → child via `parent_id`).
//! * Expired leases are removed by `tick`.

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, Utc};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeaseState {
    Active,
    Renewed,
    Revoked,
    Expired,
}

#[derive(Debug, Clone)]
pub struct Lease {
    pub id: String,
    pub parent_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub ttl_secs: i64,
    pub max_ttl_secs: i64,
    pub state: LeaseState,
}

impl Lease {
    pub fn expires_at(&self) -> DateTime<Utc> {
        self.created_at + Duration::seconds(self.ttl_secs)
    }
    pub fn max_expires_at(&self) -> DateTime<Utc> {
        self.created_at + Duration::seconds(self.max_ttl_secs)
    }
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        now >= self.expires_at()
    }
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum LeaseError {
    #[error("lease not found: {0}")]
    NotFound(String),
    #[error("lease already revoked: {0}")]
    AlreadyRevoked(String),
    #[error("renewal exceeds max ttl")]
    RenewalExceedsMax,
    #[error("invalid ttl: {0}")]
    InvalidTtl(i64),
}

#[derive(Debug, Default)]
pub struct LeaseEngine {
    leases: BTreeMap<String, Lease>,
}

impl LeaseEngine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create(
        &mut self,
        id: &str,
        parent_id: Option<&str>,
        ttl_secs: i64,
        max_ttl_secs: i64,
        now: DateTime<Utc>,
    ) -> Result<(), LeaseError> {
        if ttl_secs <= 0 || max_ttl_secs <= 0 || ttl_secs > max_ttl_secs {
            return Err(LeaseError::InvalidTtl(ttl_secs));
        }
        self.leases.insert(
            id.to_string(),
            Lease {
                id: id.to_string(),
                parent_id: parent_id.map(String::from),
                created_at: now,
                ttl_secs,
                max_ttl_secs,
                state: LeaseState::Active,
            },
        );
        Ok(())
    }

    pub fn renew(
        &mut self,
        id: &str,
        increment_secs: i64,
        now: DateTime<Utc>,
    ) -> Result<DateTime<Utc>, LeaseError> {
        let lease = self
            .leases
            .get_mut(id)
            .ok_or_else(|| LeaseError::NotFound(id.into()))?;
        if lease.state == LeaseState::Revoked {
            return Err(LeaseError::AlreadyRevoked(id.into()));
        }
        let proposed_expiry = now + Duration::seconds(increment_secs);
        let max = lease.max_expires_at();
        let new_expiry = if proposed_expiry > max {
            max
        } else {
            proposed_expiry
        };
        let elapsed = now - lease.created_at;
        let new_ttl = (new_expiry - lease.created_at).num_seconds();
        if new_ttl <= elapsed.num_seconds() {
            return Err(LeaseError::RenewalExceedsMax);
        }
        lease.ttl_secs = new_ttl;
        lease.state = LeaseState::Renewed;
        Ok(lease.expires_at())
    }

    pub fn revoke(&mut self, id: &str) -> Result<usize, LeaseError> {
        let mut to_revoke = vec![id.to_string()];
        let mut visited = Vec::new();
        while let Some(current) = to_revoke.pop() {
            if !self.leases.contains_key(&current) {
                continue;
            }
            visited.push(current.clone());
            // Find children
            for (lid, lease) in &self.leases {
                if lease.parent_id.as_deref() == Some(&current) && !visited.contains(lid) {
                    to_revoke.push(lid.clone());
                }
            }
        }
        if visited.is_empty() {
            return Err(LeaseError::NotFound(id.into()));
        }
        for lid in &visited {
            if let Some(l) = self.leases.get_mut(lid) {
                l.state = LeaseState::Revoked;
            }
        }
        Ok(visited.len())
    }

    pub fn tick(&mut self, now: DateTime<Utc>) -> Vec<String> {
        let mut expired = Vec::new();
        for (id, lease) in self.leases.iter_mut() {
            if lease.state == LeaseState::Active || lease.state == LeaseState::Renewed {
                if lease.is_expired(now) {
                    lease.state = LeaseState::Expired;
                    expired.push(id.clone());
                }
            }
        }
        expired
    }

    pub fn get(&self, id: &str) -> Option<&Lease> {
        self.leases.get(id)
    }

    pub fn count(&self) -> usize {
        self.leases.len()
    }

    pub fn active_count(&self) -> usize {
        self.leases
            .values()
            .filter(|l| matches!(l.state, LeaseState::Active | LeaseState::Renewed))
            .count()
    }

    /// Children of `parent_id`, recursively.
    pub fn descendants(&self, parent_id: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut frontier = vec![parent_id.to_string()];
        while let Some(p) = frontier.pop() {
            for (id, l) in &self.leases {
                if l.parent_id.as_deref() == Some(&p) && !out.contains(id) {
                    out.push(id.clone());
                    frontier.push(id.clone());
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t() -> DateTime<Utc> {
        Utc::now()
    }

    #[test]
    fn create_active_lease() {
        let mut e = LeaseEngine::new();
        e.create("l1", None, 60, 3600, t()).unwrap();
        assert_eq!(e.get("l1").unwrap().state, LeaseState::Active);
    }

    #[test]
    fn create_invalid_ttl_errors() {
        let mut e = LeaseEngine::new();
        let err = e.create("l1", None, 0, 100, t()).unwrap_err();
        assert_eq!(err, LeaseError::InvalidTtl(0));
    }

    #[test]
    fn create_ttl_exceeds_max_errors() {
        let mut e = LeaseEngine::new();
        let err = e.create("l1", None, 200, 100, t()).unwrap_err();
        assert!(matches!(err, LeaseError::InvalidTtl(_)));
    }

    #[test]
    fn renew_extends_ttl() {
        let mut e = LeaseEngine::new();
        let now = t();
        e.create("l1", None, 60, 3600, now).unwrap();
        let new_exp = e.renew("l1", 600, now).unwrap();
        assert!(new_exp > now + Duration::seconds(60));
    }

    #[test]
    fn renew_capped_at_max_ttl() {
        let mut e = LeaseEngine::new();
        let now = t();
        e.create("l1", None, 60, 100, now).unwrap();
        let new_exp = e.renew("l1", 1000, now).unwrap();
        assert_eq!(new_exp, now + Duration::seconds(100));
    }

    #[test]
    fn renew_revoked_errors() {
        let mut e = LeaseEngine::new();
        let now = t();
        e.create("l1", None, 60, 3600, now).unwrap();
        e.revoke("l1").unwrap();
        let err = e.renew("l1", 60, now).unwrap_err();
        assert_eq!(err, LeaseError::AlreadyRevoked("l1".into()));
    }

    #[test]
    fn renew_unknown_errors() {
        let mut e = LeaseEngine::new();
        let err = e.renew("nope", 60, t()).unwrap_err();
        assert_eq!(err, LeaseError::NotFound("nope".into()));
    }

    #[test]
    fn revoke_marks_state() {
        let mut e = LeaseEngine::new();
        e.create("l1", None, 60, 3600, t()).unwrap();
        let n = e.revoke("l1").unwrap();
        assert_eq!(n, 1);
        assert_eq!(e.get("l1").unwrap().state, LeaseState::Revoked);
    }

    #[test]
    fn revoke_cascades_to_children() {
        let mut e = LeaseEngine::new();
        let now = t();
        e.create("p", None, 60, 3600, now).unwrap();
        e.create("c1", Some("p"), 60, 3600, now).unwrap();
        e.create("c2", Some("p"), 60, 3600, now).unwrap();
        e.create("g", Some("c1"), 60, 3600, now).unwrap();
        let n = e.revoke("p").unwrap();
        assert_eq!(n, 4);
        assert_eq!(e.get("g").unwrap().state, LeaseState::Revoked);
    }

    #[test]
    fn revoke_unknown_errors() {
        let mut e = LeaseEngine::new();
        let err = e.revoke("nope").unwrap_err();
        assert_eq!(err, LeaseError::NotFound("nope".into()));
    }

    #[test]
    fn tick_expires_past_leases() {
        let mut e = LeaseEngine::new();
        let now = t();
        e.create("l1", None, 60, 3600, now).unwrap();
        let later = now + Duration::seconds(120);
        let expired = e.tick(later);
        assert_eq!(expired, vec!["l1".to_string()]);
        assert_eq!(e.get("l1").unwrap().state, LeaseState::Expired);
    }

    #[test]
    fn tick_does_not_expire_revoked() {
        let mut e = LeaseEngine::new();
        let now = t();
        e.create("l1", None, 60, 3600, now).unwrap();
        e.revoke("l1").unwrap();
        let expired = e.tick(now + Duration::seconds(120));
        assert!(expired.is_empty());
    }

    #[test]
    fn active_count_reflects_state() {
        let mut e = LeaseEngine::new();
        let now = t();
        e.create("a", None, 60, 3600, now).unwrap();
        e.create("b", None, 60, 3600, now).unwrap();
        e.revoke("b").unwrap();
        assert_eq!(e.active_count(), 1);
    }

    #[test]
    fn descendants_returns_all_children() {
        let mut e = LeaseEngine::new();
        let now = t();
        e.create("p", None, 60, 3600, now).unwrap();
        e.create("c1", Some("p"), 60, 3600, now).unwrap();
        e.create("c2", Some("p"), 60, 3600, now).unwrap();
        e.create("g", Some("c1"), 60, 3600, now).unwrap();
        let mut d = e.descendants("p");
        d.sort();
        assert_eq!(d, vec!["c1", "c2", "g"]);
    }

    #[test]
    fn lease_expires_at_matches_ttl() {
        let mut e = LeaseEngine::new();
        let now = t();
        e.create("l1", None, 100, 3600, now).unwrap();
        let l = e.get("l1").unwrap();
        assert_eq!(l.expires_at(), now + Duration::seconds(100));
    }

    #[test]
    fn lease_max_expires_at_matches_max_ttl() {
        let mut e = LeaseEngine::new();
        let now = t();
        e.create("l1", None, 100, 3600, now).unwrap();
        let l = e.get("l1").unwrap();
        assert_eq!(l.max_expires_at(), now + Duration::seconds(3600));
    }
}

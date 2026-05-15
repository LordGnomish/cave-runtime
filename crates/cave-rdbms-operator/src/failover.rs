// SPDX-License-Identifier: AGPL-3.0-or-later
//
// Source: cloudnative-pg/cloudnative-pg@v1.24.0 internal/controller/cluster_status.go
// Source: cloudnative-pg/cloudnative-pg@v1.24.0 pkg/management/postgres/promotion.go
//
//! HA failover state machine — extends `ha.rs` with leader election (via
//! lease), `pg_ctl promote` invocation, demotion of recovered primary, and
//! split-brain detection via fencing token.
//!
//! The leader election is *lease-based* and pluggable: a `Lease` trait
//! abstracts the underlying store (real deployments wire it to cave-etcd
//! via `cave-etcd::raft_bridge`; tests use `InMemoryLease`).

use crate::error::{PgError, PgResult};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

/// Fencing token: monotonic; incremented on every primary change. A late-
/// arriving demoted primary that tries to write with an old token is rejected.
pub type FencingToken = u64;

/// Lease holder — the elected primary's instance id + the token it owns.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LeaseHolder {
    pub cluster: String,
    pub instance_id: String,
    pub token: FencingToken,
    pub acquired_at: DateTime<Utc>,
    pub ttl_seconds: u32,
}

impl LeaseHolder {
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        now > self.acquired_at + Duration::seconds(i64::from(self.ttl_seconds))
    }
}

/// Generic lease store. Implementations: `InMemoryLease`, future Raft-backed.
pub trait Lease: Send + Sync {
    fn try_acquire(&self, cluster: &str, instance_id: &str, ttl_seconds: u32)
        -> Result<LeaseHolder, String>;
    fn renew(&self, cluster: &str, instance_id: &str) -> Result<LeaseHolder, String>;
    fn current(&self, cluster: &str) -> Option<LeaseHolder>;
    fn release(&self, cluster: &str, instance_id: &str) -> Result<(), String>;
}

#[derive(Default)]
pub struct InMemoryLease {
    inner: Mutex<HashMap<String, (LeaseHolder, u64)>>,
}

impl Lease for InMemoryLease {
    fn try_acquire(
        &self,
        cluster: &str,
        instance_id: &str,
        ttl_seconds: u32,
    ) -> Result<LeaseHolder, String> {
        let now = Utc::now();
        let mut inner = self.inner.lock().unwrap();
        if let Some((holder, token)) = inner.get(cluster) {
            if !holder.is_expired(now) && holder.instance_id != instance_id {
                return Err(format!(
                    "lease held by {} (token {})",
                    holder.instance_id, token
                ));
            }
            // Steal an expired lease — increments the token.
            let new_token = token + 1;
            let h = LeaseHolder {
                cluster: cluster.into(),
                instance_id: instance_id.into(),
                token: new_token,
                acquired_at: now,
                ttl_seconds,
            };
            inner.insert(cluster.into(), (h.clone(), new_token));
            return Ok(h);
        }
        let h = LeaseHolder {
            cluster: cluster.into(),
            instance_id: instance_id.into(),
            token: 1,
            acquired_at: now,
            ttl_seconds,
        };
        inner.insert(cluster.into(), (h.clone(), 1));
        Ok(h)
    }

    fn renew(&self, cluster: &str, instance_id: &str) -> Result<LeaseHolder, String> {
        let mut inner = self.inner.lock().unwrap();
        let (holder, token) = inner
            .get_mut(cluster)
            .ok_or_else(|| format!("no lease for {}", cluster))?;
        if holder.instance_id != instance_id {
            return Err(format!("lease owner mismatch: {}", holder.instance_id));
        }
        holder.acquired_at = Utc::now();
        Ok(LeaseHolder {
            token: *token,
            ..holder.clone()
        })
    }

    fn current(&self, cluster: &str) -> Option<LeaseHolder> {
        self.inner.lock().unwrap().get(cluster).map(|t| t.0.clone())
    }

    fn release(&self, cluster: &str, instance_id: &str) -> Result<(), String> {
        let mut inner = self.inner.lock().unwrap();
        if let Some((h, _)) = inner.get(cluster) {
            if h.instance_id == instance_id {
                inner.remove(cluster);
                return Ok(());
            }
        }
        Err("not the lease owner".into())
    }
}

/// `pg_ctl promote` shim. Real implementations exec on the instance pod.
pub trait PromoteRunner: Send + Sync {
    fn promote(&self, instance_id: &str) -> Result<String, String>;
    fn demote(&self, instance_id: &str, new_primary: &str) -> Result<(), String>;
}

#[derive(Default)]
pub struct MockPromoteRunner {
    pub promoted: Mutex<Vec<String>>,
    pub demoted: Mutex<Vec<(String, String)>>,
    pub fail_promote: Mutex<bool>,
    pub fail_demote: Mutex<bool>,
}

impl PromoteRunner for MockPromoteRunner {
    fn promote(&self, instance_id: &str) -> Result<String, String> {
        if std::mem::replace(&mut *self.fail_promote.lock().unwrap(), false) {
            return Err("simulated pg_ctl promote failure".into());
        }
        self.promoted.lock().unwrap().push(instance_id.into());
        Ok(format!(
            "server promoted on {}; new timeline T2",
            instance_id
        ))
    }
    fn demote(&self, instance_id: &str, new_primary: &str) -> Result<(), String> {
        if std::mem::replace(&mut *self.fail_demote.lock().unwrap(), false) {
            return Err("simulated demote failure".into());
        }
        self.demoted
            .lock()
            .unwrap()
            .push((instance_id.into(), new_primary.into()));
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum FailoverPhase {
    Pending,
    Promoting,
    Promoted,
    Failed,
    SplitBrainFenced,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FailoverRecord {
    pub id: String,
    pub cluster: String,
    pub from_primary: String,
    pub to_primary: String,
    pub reason: String,
    pub phase: FailoverPhase,
    pub fencing_token: FencingToken,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
}

pub struct FailoverController {
    lease: Arc<dyn Lease>,
    promoter: Arc<dyn PromoteRunner>,
    history: Arc<RwLock<Vec<FailoverRecord>>>,
}

impl FailoverController {
    pub fn new(lease: Arc<dyn Lease>, promoter: Arc<dyn PromoteRunner>) -> Self {
        Self {
            lease,
            promoter,
            history: Arc::new(RwLock::new(Vec::new())),
        }
    }

    pub fn current_primary(&self, cluster: &str) -> Option<LeaseHolder> {
        self.lease.current(cluster)
    }

    pub fn history(&self) -> Vec<FailoverRecord> {
        self.history.read().unwrap().clone()
    }

    pub fn history_for(&self, cluster: &str) -> Vec<FailoverRecord> {
        self.history
            .read()
            .unwrap()
            .iter()
            .filter(|r| r.cluster == cluster)
            .cloned()
            .collect()
    }

    /// Promote `candidate` to primary. Must succeed in acquiring the lease.
    pub fn promote(
        &self,
        cluster: &str,
        from_primary: &str,
        candidate: &str,
        reason: &str,
        ttl_seconds: u32,
    ) -> PgResult<FailoverRecord> {
        let now = Utc::now();
        let mut rec = FailoverRecord {
            id: uuid::Uuid::new_v4().to_string(),
            cluster: cluster.into(),
            from_primary: from_primary.into(),
            to_primary: candidate.into(),
            reason: reason.into(),
            phase: FailoverPhase::Promoting,
            fencing_token: 0,
            started_at: now,
            completed_at: None,
            last_error: None,
        };

        // 1. Acquire the lease (this bumps the fencing token).
        let holder = self
            .lease
            .try_acquire(cluster, candidate, ttl_seconds)
            .map_err(|e| {
                rec.phase = FailoverPhase::Failed;
                rec.last_error = Some(e.clone());
                rec.completed_at = Some(Utc::now());
                self.history.write().unwrap().push(rec.clone());
                PgError::ReplicationError(format!("lease acquire failed: {}", e))
            })?;
        rec.fencing_token = holder.token;

        // 2. Promote on the PG side.
        if let Err(e) = self.promoter.promote(candidate) {
            // Release the lease — promotion failed.
            let _ = self.lease.release(cluster, candidate);
            rec.phase = FailoverPhase::Failed;
            rec.last_error = Some(e.clone());
            rec.completed_at = Some(Utc::now());
            self.history.write().unwrap().push(rec.clone());
            return Err(PgError::ReplicationError(e));
        }

        // 3. Best-effort demote of the previous primary.
        let _ = self.promoter.demote(from_primary, candidate);

        rec.phase = FailoverPhase::Promoted;
        rec.completed_at = Some(Utc::now());
        self.history.write().unwrap().push(rec.clone());
        Ok(rec)
    }

    /// Detect split-brain: an old primary returns with a token < current.
    /// Returns Some(reason) if fenced. The caller should reject any writes
    /// from this instance.
    pub fn check_fencing(
        &self,
        cluster: &str,
        instance_id: &str,
        presented_token: FencingToken,
    ) -> Option<String> {
        let current = self.lease.current(cluster)?;
        if presented_token < current.token {
            return Some(format!(
                "fenced: {} presented token {} but current is {} (owner {})",
                instance_id, presented_token, current.token, current.instance_id
            ));
        }
        None
    }

    /// Process a rejoining old primary — emits demote and records a fenced
    /// event if the token is stale.
    pub fn handle_rejoin(
        &self,
        cluster: &str,
        old_primary: &str,
        presented_token: FencingToken,
    ) -> PgResult<()> {
        let current = self
            .lease
            .current(cluster)
            .ok_or_else(|| PgError::ReplicationError(format!("no leader for {}", cluster)))?;
        if presented_token < current.token {
            // Fence — record an event in history.
            let rec = FailoverRecord {
                id: uuid::Uuid::new_v4().to_string(),
                cluster: cluster.into(),
                from_primary: old_primary.into(),
                to_primary: current.instance_id.clone(),
                reason: format!(
                    "split-brain fence: stale token {} < {}",
                    presented_token, current.token
                ),
                phase: FailoverPhase::SplitBrainFenced,
                fencing_token: current.token,
                started_at: Utc::now(),
                completed_at: Some(Utc::now()),
                last_error: None,
            };
            self.history.write().unwrap().push(rec);
            return Err(PgError::ReplicationError(format!(
                "instance {} fenced — must rejoin as replica",
                old_primary
            )));
        }
        // Token matches; demote the old primary to replica.
        self.promoter
            .demote(old_primary, &current.instance_id)
            .map_err(PgError::ReplicationError)?;
        Ok(())
    }

    /// Renew lease (call on a timer from the elected primary).
    pub fn renew(&self, cluster: &str, instance_id: &str) -> PgResult<LeaseHolder> {
        self.lease
            .renew(cluster, instance_id)
            .map_err(PgError::ReplicationError)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixt() -> (FailoverController, Arc<InMemoryLease>, Arc<MockPromoteRunner>) {
        let lease: Arc<InMemoryLease> = Arc::new(InMemoryLease::default());
        let prom = Arc::new(MockPromoteRunner::default());
        let c = FailoverController::new(lease.clone(), prom.clone());
        (c, lease, prom)
    }

    #[test]
    fn lease_first_acquire_starts_at_token_1() {
        let (c, _, _) = fixt();
        c.promote("c1", "pg-0", "pg-1", "manual", 30).unwrap();
        let lh = c.current_primary("c1").unwrap();
        assert_eq!(lh.token, 1);
        assert_eq!(lh.instance_id, "pg-1");
    }

    #[test]
    fn lease_cannot_steal_active_lease() {
        let (c, _, _) = fixt();
        c.promote("c1", "pg-0", "pg-1", "manual", 30).unwrap();
        let err = c
            .promote("c1", "pg-1", "pg-2", "second attempt", 30)
            .unwrap_err();
        assert!(err.to_string().contains("lease"));
    }

    #[test]
    fn lease_steal_after_expiry_bumps_token() {
        let (c, lease, _) = fixt();
        c.promote("c1", "pg-0", "pg-1", "manual", 0).unwrap(); // 0s TTL = expired
        std::thread::sleep(std::time::Duration::from_millis(50));
        // Expired lease can be stolen.
        let h = lease.try_acquire("c1", "pg-2", 30).unwrap();
        assert_eq!(h.token, 2);
        assert_eq!(h.instance_id, "pg-2");
    }

    #[test]
    fn promote_calls_pg_ctl_and_demotes_previous() {
        let (c, _, prom) = fixt();
        c.promote("c1", "pg-0", "pg-1", "manual", 30).unwrap();
        assert!(prom.promoted.lock().unwrap().iter().any(|x| x == "pg-1"));
        assert!(prom
            .demoted
            .lock()
            .unwrap()
            .iter()
            .any(|(a, b)| a == "pg-0" && b == "pg-1"));
    }

    #[test]
    fn promote_failure_releases_lease_and_records_failed() {
        let (c, lease, prom) = fixt();
        *prom.fail_promote.lock().unwrap() = true;
        let err = c.promote("c1", "pg-0", "pg-1", "manual", 30).unwrap_err();
        assert!(err.to_string().contains("promote"));
        // No lease held.
        assert!(lease.current("c1").is_none());
        // History recorded as Failed.
        assert_eq!(c.history().len(), 1);
        assert_eq!(c.history()[0].phase, FailoverPhase::Failed);
    }

    #[test]
    fn fencing_rejects_stale_token() {
        let (c, _, _) = fixt();
        c.promote("c1", "pg-0", "pg-1", "manual", 60).unwrap(); // token=1
        // Old primary returns with token 0.
        let reason = c.check_fencing("c1", "pg-0", 0).unwrap();
        assert!(reason.contains("fenced"));
    }

    #[test]
    fn fencing_passes_current_token() {
        let (c, _, _) = fixt();
        c.promote("c1", "pg-0", "pg-1", "manual", 60).unwrap();
        assert!(c.check_fencing("c1", "pg-1", 1).is_none());
    }

    #[test]
    fn rejoin_fences_stale_primary() {
        let (c, _, prom) = fixt();
        c.promote("c1", "pg-0", "pg-1", "manual", 60).unwrap();
        let err = c.handle_rejoin("c1", "pg-0", 0).unwrap_err();
        assert!(err.to_string().contains("fenced"));
        // Recorded as fenced event in history.
        assert!(c
            .history()
            .iter()
            .any(|r| r.phase == FailoverPhase::SplitBrainFenced));
        // No additional demote (the demote during promote happened, but
        // rejoin path didn't add one).
        assert_eq!(prom.demoted.lock().unwrap().len(), 1);
    }

    #[test]
    fn rejoin_with_current_token_demotes_to_replica() {
        let (c, _, prom) = fixt();
        c.promote("c1", "pg-0", "pg-1", "manual", 60).unwrap();
        prom.demoted.lock().unwrap().clear();
        c.handle_rejoin("c1", "pg-3", 1).unwrap();
        let demoted = prom.demoted.lock().unwrap();
        assert!(demoted.iter().any(|(a, _)| a == "pg-3"));
    }

    #[test]
    fn renew_keeps_token_stable() {
        let (c, _, _) = fixt();
        c.promote("c1", "pg-0", "pg-1", "manual", 30).unwrap();
        let h = c.renew("c1", "pg-1").unwrap();
        assert_eq!(h.token, 1);
    }

    #[test]
    fn renew_by_non_owner_fails() {
        let (c, _, _) = fixt();
        c.promote("c1", "pg-0", "pg-1", "manual", 30).unwrap();
        assert!(c.renew("c1", "pg-2").is_err());
    }

    #[test]
    fn history_filters_by_cluster() {
        let (c, _, _) = fixt();
        c.promote("c1", "pg-0", "pg-1", "manual", 30).unwrap();
        c.promote("c2", "pg-10", "pg-11", "manual", 30).unwrap();
        assert_eq!(c.history_for("c1").len(), 1);
        assert_eq!(c.history_for("c2").len(), 1);
        assert_eq!(c.history().len(), 2);
    }

    #[test]
    fn lease_release_clears_holder() {
        let (_c, lease, _) = fixt();
        lease.try_acquire("c1", "pg-1", 30).unwrap();
        lease.release("c1", "pg-1").unwrap();
        assert!(lease.current("c1").is_none());
    }
}

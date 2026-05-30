// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Notification log (nflog) — pure in-memory core.
//!
//! Faithful line-port of the in-memory algorithm in upstream
//! prometheus/alertmanager v0.26.0 `nflog/nflog.go`. The notification log
//! records, for each (receiver, group_key) pair, the most recent set of
//! firing/resolved alert fingerprints that were notified, so the dispatcher
//! can decide whether a group's *contents* changed since the last send
//! (feeding `group_interval` / `repeat_interval`).
//!
//! Ported (pure, in-memory):
//!   - `receiverKey` / `stateKey`              (nflog.go:366-374)
//!   - `(*Log).Log` newer-timestamp-wins write (nflog.go:376-416)
//!   - `(*Log).Query` most-recent entry lookup (nflog.go:443-475)
//!   - `(*Log).GC` expiry by `ExpiresAt`       (nflog.go:419-440)
//!   - `state.merge` CRDT last-writer-wins      (nflog.go:178-190)
//!
//! Scope-cut (stays out of crate, see parity.manifest.toml):
//!   - Snapshot / loadSnapshot / Maintenance WAL persistence → cave-etcd HA.
//!   - Gossip `broadcast` transport / `MarshalBinary` protobuf wire format →
//!     architecturally superseded by cave-etcd Raft (ADR-RUNTIME-STACK-001).

use chrono::{DateTime, Duration, Utc};
use std::collections::HashMap;

/// Errors surfaced by the notification log. Mirrors upstream
/// `ErrNotFound` (nflog.go:41).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NflogError {
    /// No entry for the queried (receiver, group_key). Upstream `ErrNotFound`.
    #[error("not found")]
    NotFound,
}

/// Identifies a notification target. Mirrors the fields of upstream
/// `nflogpb.Receiver` that participate in key derivation
/// (`GroupName`, `Integration`, `Idx`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NflogReceiver {
    /// Route group name the receiver belongs to.
    pub group_name: String,
    /// Integration kind (e.g. `slack`, `webhook`, `pagerduty`).
    pub integration: String,
    /// Index of this integration within the receiver.
    pub idx: u32,
}

impl NflogReceiver {
    /// `receiverKey` — `fmt.Sprintf("%s/%s/%d", GroupName, Integration, Idx)`
    /// (nflog.go:366-368).
    pub fn receiver_key(&self) -> String {
        format!("{}/{}/{}", self.group_name, self.integration, self.idx)
    }
}

/// A recorded notification-log entry. Mirrors `nflogpb.Entry` paired with the
/// `MeshEntry.ExpiresAt` envelope used for GC and merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeshEntry {
    /// Target receiver.
    pub receiver: NflogReceiver,
    /// Group key the notification was sent for.
    pub group_key: String,
    /// When the notification was recorded.
    pub timestamp: DateTime<Utc>,
    /// Fingerprints (as u64 hashes) of alerts that were firing.
    pub firing_alerts: Vec<u64>,
    /// Fingerprints of alerts that were resolved.
    pub resolved_alerts: Vec<u64>,
    /// When this entry becomes eligible for GC.
    pub expires_at: DateTime<Utc>,
}

/// In-memory notification log. The key is `stateKey(group_key, receiver)`;
/// for now (matching upstream) only the most-recent entry per key is kept.
#[derive(Debug, Default)]
pub struct NotificationLog {
    st: HashMap<String, MeshEntry>,
    retention: Duration,
}

impl NotificationLog {
    /// Construct an empty log with the given retention window. `retention`
    /// mirrors `Options.Retention` (nflog.go:236).
    pub fn new(retention: Duration) -> Self {
        Self {
            st: HashMap::new(),
            retention,
        }
    }

    /// `stateKey` — `fmt.Sprintf("%s:%s", group_key, receiverKey(r))`
    /// (nflog.go:372-374).
    pub fn state_key(group_key: &str, r: &NflogReceiver) -> String {
        format!("{}:{}", group_key, r.receiver_key())
    }

    /// `(*Log).Log` with the clock supplied by the caller (nflog.go:376-416).
    ///
    /// `expiry` of `Some(d)` shortens the entry's lifetime when
    /// `retention > d` (matches `if expiry > 0 && l.retention > expiry`).
    /// Honours the clock-drift guard: a write whose `now` is not after an
    /// existing entry's timestamp is ignored.
    pub fn log_at(
        &mut self,
        r: &NflogReceiver,
        group_key: &str,
        firing_alerts: Vec<u64>,
        resolved_alerts: Vec<u64>,
        expiry: Option<Duration>,
        now: DateTime<Utc>,
    ) -> Result<(), NflogError> {
        let key = Self::state_key(group_key, r);

        if let Some(prev) = self.st.get(&key) {
            // Entry already exists, only overwrite if timestamp is newer.
            // This may happen with raciness or clock-drift across nodes.
            if prev.timestamp > now {
                return Ok(());
            }
        }

        let mut expires_at = now + self.retention;
        if let Some(exp) = expiry {
            if exp > Duration::zero() && self.retention > exp {
                expires_at = now + exp;
            }
        }

        let e = MeshEntry {
            receiver: r.clone(),
            group_key: group_key.to_string(),
            timestamp: now,
            firing_alerts,
            resolved_alerts,
            expires_at,
        };

        // l.st.merge(e, l.now()) — record the entry. broadcast() is scope-cut.
        self.merge(e, now);
        Ok(())
    }

    /// Convenience wrapper for `log_at` using the wall clock (`Utc::now`),
    /// mirroring `(*Log).Log` which reads `l.now()` (nflog.go:378).
    pub fn log(
        &mut self,
        r: &NflogReceiver,
        group_key: &str,
        firing_alerts: Vec<u64>,
        resolved_alerts: Vec<u64>,
        expiry: Option<Duration>,
    ) -> Result<(), NflogError> {
        self.log_at(r, group_key, firing_alerts, resolved_alerts, expiry, Utc::now())
    }

    /// `(*Log).Query` — return the most-recent entry for the
    /// (receiver, group_key) pair, or `NotFound` (nflog.go:443-475).
    pub fn query(&self, r: &NflogReceiver, group_key: &str) -> Result<&MeshEntry, NflogError> {
        let key = Self::state_key(group_key, r);
        self.st.get(&key).ok_or(NflogError::NotFound)
    }

    /// `(*Log).GC` with the clock supplied by the caller (nflog.go:419-440).
    /// Removes every entry whose `expires_at` is not after `now`; returns the
    /// number removed.
    pub fn gc_at(&mut self, now: DateTime<Utc>) -> Result<usize, NflogError> {
        let before = self.st.len();
        // `!le.ExpiresAt.After(now)` => expires_at <= now.
        self.st.retain(|_, le| le.expires_at > now);
        Ok(before - self.st.len())
    }

    /// `state.merge` — last-writer-wins reconciliation (nflog.go:178-190).
    ///
    /// Returns `true` when the entry was merged (inserted/updated), `false`
    /// otherwise. Upstream uses the bool to decide whether to gossip further;
    /// here it is purely informational. An entry already expired at `now` is
    /// never merged.
    pub fn merge(&mut self, e: MeshEntry, now: DateTime<Utc>) -> bool {
        if e.expires_at < now {
            return false;
        }
        let k = Self::state_key(&e.group_key, &e.receiver);
        match self.st.get(&k) {
            Some(prev) if !(prev.timestamp < e.timestamp) => false,
            _ => {
                self.st.insert(k, e);
                true
            }
        }
    }

    /// Number of entries currently held (test/inspection helper).
    pub fn len(&self) -> usize {
        self.st.len()
    }

    /// Whether the log holds no entries.
    pub fn is_empty(&self) -> bool {
        self.st.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r() -> NflogReceiver {
        NflogReceiver {
            group_name: "g".into(),
            integration: "webhook".into(),
            idx: 0,
        }
    }

    #[test]
    fn keys_compose() {
        assert_eq!(r().receiver_key(), "g/webhook/0");
        assert_eq!(NotificationLog::state_key("gk", &r()), "gk:g/webhook/0");
    }

    #[test]
    fn log_query_roundtrip() {
        let mut l = NotificationLog::new(Duration::hours(1));
        l.log(&r(), "gk", vec![1], vec![], None).unwrap();
        assert_eq!(l.query(&r(), "gk").unwrap().firing_alerts, vec![1]);
        assert_eq!(l.len(), 1);
    }
}

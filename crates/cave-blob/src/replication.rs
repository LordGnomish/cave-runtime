//! Bucket replication — async cross-bucket / cross-site object copy.
//!
//! Mirrors S3 / MinIO bucket-replication configuration:
//!
//! * Per-rule filter (key prefix or tag).
//! * Per-rule destination bucket + storage class.
//! * Per-rule priority (highest wins on overlap).
//! * Per-object replication status (PENDING / COMPLETED / FAILED / REPLICA).

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ReplicationError {
    #[error("rule not found: {0}")]
    RuleNotFound(String),
    #[error("invalid priority: {0}")]
    InvalidPriority(i32),
    #[error("missing destination")]
    MissingDestination,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplicationStatus {
    Pending,
    Completed,
    Failed,
    Replica,
}

impl ReplicationStatus {
    pub fn header_value(self) -> &'static str {
        match self {
            Self::Pending => "PENDING",
            Self::Completed => "COMPLETED",
            Self::Failed => "FAILED",
            Self::Replica => "REPLICA",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicationRule {
    pub id: String,
    pub priority: i32,
    pub enabled: bool,
    pub prefix: String,
    pub destination_bucket: String,
    pub destination_storage_class: Option<String>,
    pub delete_marker_replication: bool,
}

impl ReplicationRule {
    pub fn new(id: &str, priority: i32, prefix: &str, destination: &str) -> Result<Self, ReplicationError> {
        if priority < 0 {
            return Err(ReplicationError::InvalidPriority(priority));
        }
        if destination.is_empty() {
            return Err(ReplicationError::MissingDestination);
        }
        Ok(Self {
            id: id.into(),
            priority,
            enabled: true,
            prefix: prefix.into(),
            destination_bucket: destination.into(),
            destination_storage_class: None,
            delete_marker_replication: false,
        })
    }

    pub fn matches(&self, key: &str) -> bool {
        self.enabled && key.starts_with(&self.prefix)
    }
}

#[derive(Debug, Default)]
pub struct ReplicationConfig {
    pub rules: Vec<ReplicationRule>,
}

impl ReplicationConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_rule(&mut self, rule: ReplicationRule) -> Result<(), ReplicationError> {
        if self.rules.iter().any(|r| r.id == rule.id) {
            return Err(ReplicationError::InvalidPriority(rule.priority));
        }
        self.rules.push(rule);
        self.rules.sort_by(|a, b| b.priority.cmp(&a.priority));
        Ok(())
    }

    pub fn drop_rule(&mut self, id: &str) -> bool {
        let len = self.rules.len();
        self.rules.retain(|r| r.id != id);
        self.rules.len() < len
    }

    pub fn matched_rule(&self, key: &str) -> Option<&ReplicationRule> {
        self.rules.iter().find(|r| r.matches(key))
    }

    pub fn count(&self) -> usize {
        self.rules.len()
    }
}

#[derive(Debug, Clone)]
pub struct ReplicationJob {
    pub source_bucket: String,
    pub source_key: String,
    pub destination_bucket: String,
    pub status: ReplicationStatus,
    pub last_attempt: DateTime<Utc>,
    pub attempt_count: u32,
    pub last_error: Option<String>,
}

#[derive(Debug, Default)]
pub struct ReplicationQueue {
    pub jobs: BTreeMap<(String, String), ReplicationJob>,
}

impl ReplicationQueue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn enqueue(
        &mut self,
        source_bucket: &str,
        source_key: &str,
        destination_bucket: &str,
        now: DateTime<Utc>,
    ) {
        let job = ReplicationJob {
            source_bucket: source_bucket.into(),
            source_key: source_key.into(),
            destination_bucket: destination_bucket.into(),
            status: ReplicationStatus::Pending,
            last_attempt: now,
            attempt_count: 0,
            last_error: None,
        };
        self.jobs.insert((source_bucket.into(), source_key.into()), job);
    }

    pub fn mark_complete(&mut self, source_bucket: &str, source_key: &str, now: DateTime<Utc>) -> Option<()> {
        let job = self.jobs.get_mut(&(source_bucket.into(), source_key.into()))?;
        job.status = ReplicationStatus::Completed;
        job.last_attempt = now;
        Some(())
    }

    pub fn mark_failed(
        &mut self,
        source_bucket: &str,
        source_key: &str,
        reason: &str,
        now: DateTime<Utc>,
    ) -> Option<()> {
        let job = self.jobs.get_mut(&(source_bucket.into(), source_key.into()))?;
        job.status = ReplicationStatus::Failed;
        job.last_attempt = now;
        job.attempt_count += 1;
        job.last_error = Some(reason.into());
        Some(())
    }

    pub fn pending(&self) -> Vec<&ReplicationJob> {
        self.jobs
            .values()
            .filter(|j| j.status == ReplicationStatus::Pending)
            .collect()
    }

    pub fn pending_count(&self) -> usize {
        self.pending().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_matches_prefix() {
        let r = ReplicationRule::new("r1", 1, "logs/", "dest").unwrap();
        assert!(r.matches("logs/2026/x.txt"));
        assert!(!r.matches("data/x.txt"));
    }

    #[test]
    fn rule_disabled_does_not_match() {
        let mut r = ReplicationRule::new("r1", 1, "", "dest").unwrap();
        r.enabled = false;
        assert!(!r.matches("anything"));
    }

    #[test]
    fn rule_invalid_priority_errors() {
        let err = ReplicationRule::new("r1", -5, "", "dest").unwrap_err();
        assert!(matches!(err, ReplicationError::InvalidPriority(_)));
    }

    #[test]
    fn rule_missing_destination_errors() {
        let err = ReplicationRule::new("r1", 1, "", "").unwrap_err();
        assert_eq!(err, ReplicationError::MissingDestination);
    }

    #[test]
    fn config_sorts_by_priority_desc() {
        let mut c = ReplicationConfig::new();
        c.add_rule(ReplicationRule::new("low", 1, "", "d").unwrap()).unwrap();
        c.add_rule(ReplicationRule::new("hi", 100, "", "d").unwrap()).unwrap();
        c.add_rule(ReplicationRule::new("mid", 50, "", "d").unwrap()).unwrap();
        assert_eq!(c.rules[0].id, "hi");
        assert_eq!(c.rules[2].id, "low");
    }

    #[test]
    fn config_matches_highest_priority_first() {
        let mut c = ReplicationConfig::new();
        c.add_rule(ReplicationRule::new("logs", 10, "logs/", "d1").unwrap()).unwrap();
        c.add_rule(ReplicationRule::new("all", 1, "", "d2").unwrap()).unwrap();
        let m = c.matched_rule("logs/x").unwrap();
        assert_eq!(m.id, "logs");
        let m2 = c.matched_rule("data/y").unwrap();
        assert_eq!(m2.id, "all");
    }

    #[test]
    fn drop_rule_removes() {
        let mut c = ReplicationConfig::new();
        c.add_rule(ReplicationRule::new("r1", 1, "", "d").unwrap()).unwrap();
        assert!(c.drop_rule("r1"));
        assert!(!c.drop_rule("r1"));
    }

    #[test]
    fn enqueue_then_mark_complete() {
        let mut q = ReplicationQueue::new();
        q.enqueue("src", "k", "dst", Utc::now());
        assert_eq!(q.pending_count(), 1);
        q.mark_complete("src", "k", Utc::now()).unwrap();
        assert_eq!(q.pending_count(), 0);
    }

    #[test]
    fn mark_failed_increments_attempt_count() {
        let mut q = ReplicationQueue::new();
        q.enqueue("src", "k", "dst", Utc::now());
        q.mark_failed("src", "k", "network", Utc::now()).unwrap();
        let job = &q.jobs[&("src".into(), "k".into())];
        assert_eq!(job.status, ReplicationStatus::Failed);
        assert_eq!(job.attempt_count, 1);
    }

    #[test]
    fn status_header_values_match_spec() {
        assert_eq!(ReplicationStatus::Pending.header_value(), "PENDING");
        assert_eq!(ReplicationStatus::Completed.header_value(), "COMPLETED");
        assert_eq!(ReplicationStatus::Replica.header_value(), "REPLICA");
    }

    #[test]
    fn pending_excludes_completed() {
        let mut q = ReplicationQueue::new();
        q.enqueue("src", "a", "dst", Utc::now());
        q.enqueue("src", "b", "dst", Utc::now());
        q.mark_complete("src", "a", Utc::now()).unwrap();
        let pending = q.pending();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].source_key, "b");
    }
}

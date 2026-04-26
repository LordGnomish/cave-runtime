//! Audit log query — `cavectl audit log --filter`.
//!
//! cave-audit query layer subset. Filter mini-DSL: `key=value` ya da `key~regex`
//! (substring), AND ile birleşir, multiple `--filter` virgül/birden çok flag.
//! Examples:
//!   cavectl audit log --filter actor=alice --filter action=tenant.suspend
//!   cavectl audit log --filter target~prod- --since 2026-04-26T00:00:00Z

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditEntry {
    pub tenant_id: String,
    pub at: DateTime<Utc>,
    pub actor: String,
    pub action: String,
    pub target: String,
    pub outcome: String,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuditFilter {
    Equals { key: String, value: String },
    Contains { key: String, needle: String },
}

impl AuditFilter {
    /// Parse `key=value` or `key~substring`. Returns the matching variant.
    pub fn parse(raw: &str) -> Result<Self> {
        if let Some((k, v)) = raw.split_once('~') {
            let key = k.trim();
            let needle = v.trim();
            if key.is_empty() || needle.is_empty() {
                return Err(anyhow!("filter '{raw}' missing key or value"));
            }
            return Ok(AuditFilter::Contains {
                key: key.to_string(),
                needle: needle.to_string(),
            });
        }
        if let Some((k, v)) = raw.split_once('=') {
            let key = k.trim();
            let value = v.trim();
            if key.is_empty() || value.is_empty() {
                return Err(anyhow!("filter '{raw}' missing key or value"));
            }
            return Ok(AuditFilter::Equals {
                key: key.to_string(),
                value: value.to_string(),
            });
        }
        Err(anyhow!(
            "filter '{raw}' must be 'key=value' or 'key~substring'"
        ))
    }

    fn field<'a>(&self, e: &'a AuditEntry) -> Option<&'a str> {
        let key = match self {
            AuditFilter::Equals { key, .. } | AuditFilter::Contains { key, .. } => key.as_str(),
        };
        match key {
            "actor" => Some(e.actor.as_str()),
            "action" => Some(e.action.as_str()),
            "target" => Some(e.target.as_str()),
            "outcome" => Some(e.outcome.as_str()),
            "tenant_id" => Some(e.tenant_id.as_str()),
            _ => None,
        }
    }

    pub fn matches(&self, e: &AuditEntry) -> bool {
        let Some(field) = self.field(e) else {
            return false;
        };
        match self {
            AuditFilter::Equals { value, .. } => field == value,
            AuditFilter::Contains { needle, .. } => field.contains(needle.as_str()),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct AuditQuery {
    pub tenant_id: String,
    pub since: Option<DateTime<Utc>>,
    pub until: Option<DateTime<Utc>>,
    pub filters: Vec<AuditFilter>,
    pub limit: Option<usize>,
}

impl AuditQuery {
    pub fn new(tenant_id: impl Into<String>) -> Self {
        Self {
            tenant_id: tenant_id.into(),
            ..Default::default()
        }
    }

    pub fn with_filter(mut self, f: AuditFilter) -> Self {
        self.filters.push(f);
        self
    }

    pub fn with_since(mut self, ts: DateTime<Utc>) -> Self {
        self.since = Some(ts);
        self
    }

    pub fn with_until(mut self, ts: DateTime<Utc>) -> Self {
        self.until = Some(ts);
        self
    }

    pub fn with_limit(mut self, n: usize) -> Self {
        self.limit = Some(n);
        self
    }

    pub fn matches(&self, e: &AuditEntry) -> bool {
        if e.tenant_id != self.tenant_id {
            return false;
        }
        if let Some(s) = self.since {
            if e.at < s {
                return false;
            }
        }
        if let Some(u) = self.until {
            if e.at > u {
                return false;
            }
        }
        self.filters.iter().all(|f| f.matches(e))
    }
}

#[async_trait]
pub trait AuditLog: Send + Sync {
    async fn append(&self, entry: AuditEntry) -> Result<()>;
    async fn query(&self, q: &AuditQuery) -> Result<Vec<AuditEntry>>;
}

#[derive(Default)]
pub struct InMemoryAuditLog {
    inner: Arc<RwLock<Vec<AuditEntry>>>,
}

impl InMemoryAuditLog {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl AuditLog for InMemoryAuditLog {
    async fn append(&self, entry: AuditEntry) -> Result<()> {
        self.inner.write().push(entry);
        Ok(())
    }

    async fn query(&self, q: &AuditQuery) -> Result<Vec<AuditEntry>> {
        let s = self.inner.read();
        let mut out: Vec<AuditEntry> = s.iter().filter(|e| q.matches(e)).cloned().collect();
        out.sort_by(|a, b| b.at.cmp(&a.at));
        if let Some(n) = q.limit {
            out.truncate(n);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn entry(tenant_id: &str, actor: &str, action: &str, target: &str) -> AuditEntry {
        AuditEntry {
            tenant_id: tenant_id.to_string(),
            at: Utc.with_ymd_and_hms(2026, 4, 26, 10, 0, 0).unwrap(),
            actor: actor.to_string(),
            action: action.to_string(),
            target: target.to_string(),
            outcome: "success".to_string(),
            metadata: serde_json::Value::Null,
        }
    }

    /// cite: cave-audit filter DSL — equals form
    #[test]
    fn audit_filter_parse_equals_form() {
        let f = AuditFilter::parse("actor=alice").unwrap();
        assert_eq!(
            f,
            AuditFilter::Equals {
                key: "actor".into(),
                value: "alice".into(),
            }
        );
    }

    /// cite: cave-audit filter DSL — contains (~) form
    #[test]
    fn audit_filter_parse_contains_form() {
        let f = AuditFilter::parse("target~prod-").unwrap();
        assert_eq!(
            f,
            AuditFilter::Contains {
                key: "target".into(),
                needle: "prod-".into(),
            }
        );
    }

    /// cite: cave-audit filter DSL — bare key without value rejected
    #[test]
    fn audit_filter_parse_bare_key_rejected() {
        let err = AuditFilter::parse("alice").unwrap_err();
        assert!(err.to_string().contains("key=value"));
    }

    /// cite: cave-audit filter DSL — empty value rejected
    #[test]
    fn audit_filter_parse_empty_value_rejected() {
        let err = AuditFilter::parse("actor=").unwrap_err();
        assert!(err.to_string().contains("missing"));
    }

    /// cite: cave-audit filter — matches actor=alice exactly
    #[tokio::test]
    async fn audit_acme_filter_actor_eq_matches_alice_only() {
        let tenant_id = "acme";
        let log = InMemoryAuditLog::new();
        log.append(entry(tenant_id, "alice", "tenant.suspend", "acme")).await.unwrap();
        log.append(entry(tenant_id, "bob", "tenant.suspend", "acme")).await.unwrap();
        let q = AuditQuery::new(tenant_id)
            .with_filter(AuditFilter::parse("actor=alice").unwrap());
        let got = log.query(&q).await.unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].actor, "alice");
    }

    /// cite: cave-audit filter — substring on target catches all prod-* targets
    #[tokio::test]
    async fn audit_acme_filter_target_contains_prod() {
        let tenant_id = "acme";
        let log = InMemoryAuditLog::new();
        log.append(entry(tenant_id, "alice", "key.rotate", "prod-db")).await.unwrap();
        log.append(entry(tenant_id, "alice", "key.rotate", "stage-db")).await.unwrap();
        log.append(entry(tenant_id, "alice", "key.rotate", "prod-cache")).await.unwrap();
        let q = AuditQuery::new(tenant_id)
            .with_filter(AuditFilter::parse("target~prod-").unwrap());
        let got = log.query(&q).await.unwrap();
        assert_eq!(got.len(), 2);
        assert!(got.iter().all(|e| e.target.starts_with("prod-")));
    }

    /// cite: cave-audit filter — multiple filters AND together
    #[tokio::test]
    async fn audit_acme_multiple_filters_and() {
        let tenant_id = "acme";
        let log = InMemoryAuditLog::new();
        log.append(entry(tenant_id, "alice", "tenant.suspend", "acme")).await.unwrap();
        log.append(entry(tenant_id, "alice", "key.rotate", "prod-db")).await.unwrap();
        log.append(entry(tenant_id, "bob", "tenant.suspend", "acme")).await.unwrap();
        let q = AuditQuery::new(tenant_id)
            .with_filter(AuditFilter::parse("actor=alice").unwrap())
            .with_filter(AuditFilter::parse("action=tenant.suspend").unwrap());
        let got = log.query(&q).await.unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].action, "tenant.suspend");
        assert_eq!(got[0].actor, "alice");
    }

    /// cite: cave-audit query — tenant_id scopes results
    #[tokio::test]
    async fn audit_query_acme_excludes_globex() {
        let log = InMemoryAuditLog::new();
        log.append(entry("acme", "alice", "x", "y")).await.unwrap();
        log.append(entry("globex", "alice", "x", "y")).await.unwrap();
        let q = AuditQuery::new("acme");
        let got = log.query(&q).await.unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].tenant_id, "acme");
    }

    /// cite: cave-audit query — `since` excludes earlier entries
    #[tokio::test]
    async fn audit_acme_since_filter_excludes_older_entries() {
        let tenant_id = "acme";
        let log = InMemoryAuditLog::new();
        let mut old = entry(tenant_id, "alice", "x", "y");
        old.at = Utc.with_ymd_and_hms(2026, 4, 25, 10, 0, 0).unwrap();
        log.append(old).await.unwrap();
        let new_e = entry(tenant_id, "alice", "x", "y");
        log.append(new_e).await.unwrap();
        let q = AuditQuery::new(tenant_id)
            .with_since(Utc.with_ymd_and_hms(2026, 4, 26, 0, 0, 0).unwrap());
        let got = log.query(&q).await.unwrap();
        assert_eq!(got.len(), 1);
    }

    /// cite: cave-audit query — limit truncates after sort by at desc
    #[tokio::test]
    async fn audit_acme_limit_truncates_results() {
        let tenant_id = "acme";
        let log = InMemoryAuditLog::new();
        for i in 0..5 {
            let mut e = entry(tenant_id, "alice", "x", "y");
            e.at = Utc.with_ymd_and_hms(2026, 4, 26, 10, i, 0).unwrap();
            log.append(e).await.unwrap();
        }
        let q = AuditQuery::new(tenant_id).with_limit(2);
        let got = log.query(&q).await.unwrap();
        assert_eq!(got.len(), 2);
        assert!(got[0].at > got[1].at, "results sorted descending");
    }

    /// cite: cave-audit filter — unknown key never matches
    #[tokio::test]
    async fn audit_acme_unknown_filter_key_returns_empty() {
        let tenant_id = "acme";
        let log = InMemoryAuditLog::new();
        log.append(entry(tenant_id, "alice", "x", "y")).await.unwrap();
        let q = AuditQuery::new(tenant_id)
            .with_filter(AuditFilter::parse("ghost=alice").unwrap());
        let got = log.query(&q).await.unwrap();
        assert!(got.is_empty());
    }
}

//! Logs — tail/search.
//!
//! The portal renders log streams natively (no Grafana Loki UI handoff). The
//! data layer ingests structured log entries and supports text-substring
//! search plus level filtering.

use std::collections::VecDeque;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::routes::rbac::{Guard, GuardError, Principal};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl LogLevel {
    pub fn rank(&self) -> u8 {
        match self {
            LogLevel::Trace => 0,
            LogLevel::Debug => 1,
            LogLevel::Info => 2,
            LogLevel::Warn => 3,
            LogLevel::Error => 4,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogEntry {
    pub id: u64,
    pub tenant: String,
    pub app: String,
    pub instance: String,
    pub level: LogLevel,
    pub message: String,
    pub timestamp: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppendLogRequest {
    pub tenant: String,
    pub app: String,
    pub instance: String,
    pub level: LogLevel,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogQuery {
    pub tenant: String,
    #[serde(default)]
    pub app: Option<String>,
    #[serde(default)]
    pub instance: Option<String>,
    #[serde(default)]
    pub min_level: Option<LogLevel>,
    #[serde(default)]
    pub contains: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub since_id: Option<u64>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum LogsError {
    #[error("guard: {0}")]
    Guard(#[from] GuardError),
    #[error("limit too large (max 5000)")]
    LimitTooLarge,
    #[error("invalid message: {0}")]
    InvalidMessage(String),
}

const RING_CAPACITY: usize = 50_000;
const DEFAULT_LIMIT: usize = 200;
const MAX_LIMIT: usize = 5_000;

pub struct LogStore {
    inner: Mutex<VecDeque<LogEntry>>,
    seq: Mutex<u64>,
}

impl Default for LogStore {
    fn default() -> Self {
        Self {
            inner: Mutex::new(VecDeque::with_capacity(RING_CAPACITY)),
            seq: Mutex::new(0),
        }
    }
}

impl LogStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn append(
        &self,
        principal: Option<&Principal>,
        req: AppendLogRequest,
    ) -> Result<LogEntry, LogsError> {
        Guard::operator_only().authorize(principal, None)?;
        if req.message.is_empty() {
            return Err(LogsError::InvalidMessage("empty".into()));
        }
        if req.message.len() > 16_384 {
            return Err(LogsError::InvalidMessage("too long".into()));
        }
        let mut seq = self.seq.lock().unwrap();
        *seq += 1;
        let id = *seq;
        drop(seq);
        let entry = LogEntry {
            id,
            tenant: req.tenant,
            app: req.app,
            instance: req.instance,
            level: req.level,
            message: req.message,
            timestamp: "1970-01-01T00:00:00Z".into(),
        };
        let mut buf = self.inner.lock().unwrap();
        if buf.len() >= RING_CAPACITY {
            buf.pop_front();
        }
        buf.push_back(entry.clone());
        Ok(entry)
    }

    pub fn query(
        &self,
        principal: Option<&Principal>,
        q: &LogQuery,
    ) -> Result<Vec<LogEntry>, LogsError> {
        Guard::cross_persona(None).authorize(principal, Some(&q.tenant))?;
        let limit = q.limit.unwrap_or(DEFAULT_LIMIT);
        if limit > MAX_LIMIT {
            return Err(LogsError::LimitTooLarge);
        }
        let buf = self.inner.lock().unwrap();
        let mut out: Vec<LogEntry> = buf
            .iter()
            .rev()
            .filter(|e| e.tenant == q.tenant)
            .filter(|e| q.app.as_deref().map_or(true, |a| e.app == a))
            .filter(|e| q.instance.as_deref().map_or(true, |i| e.instance == i))
            .filter(|e| q.min_level.map_or(true, |min| e.level.rank() >= min.rank()))
            .filter(|e| {
                q.contains
                    .as_deref()
                    .map_or(true, |needle| e.message.contains(needle))
            })
            .filter(|e| q.since_id.map_or(true, |s| e.id > s))
            .take(limit)
            .cloned()
            .collect();
        out.sort_by(|a, b| b.id.cmp(&a.id));
        Ok(out)
    }

    pub fn count(&self) -> usize {
        self.inner.lock().unwrap().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::rbac::Persona;

    fn op() -> Principal { Principal::new("o", Persona::Operator) }
    fn dev(t: &str) -> Principal { Principal::new("d", Persona::Tenant).with_tenant(t) }

    fn append(s: &LogStore, t: &str, app: &str, lvl: LogLevel, msg: &str) -> LogEntry {
        s.append(Some(&op()), AppendLogRequest {
            tenant: t.into(),
            app: app.into(),
            instance: "i-1".into(),
            level: lvl,
            message: msg.into(),
        }).unwrap()
    }

    #[test]
    fn level_rank_ordering() {
        assert!(LogLevel::Trace.rank() < LogLevel::Debug.rank());
        assert!(LogLevel::Info.rank() < LogLevel::Warn.rank());
        assert!(LogLevel::Warn.rank() < LogLevel::Error.rank());
    }

    #[test]
    fn append_anonymous_denied() {
        let s = LogStore::new();
        let err = s.append(None, AppendLogRequest {
            tenant: "acme".into(),
            app: "web".into(),
            instance: "i".into(),
            level: LogLevel::Info,
            message: "m".into(),
        }).unwrap_err();
        assert!(matches!(err, LogsError::Guard(GuardError::Anonymous)));
    }

    #[test]
    fn append_tenant_persona_denied() {
        let s = LogStore::new();
        let err = s.append(Some(&dev("acme")), AppendLogRequest {
            tenant: "acme".into(),
            app: "web".into(),
            instance: "i".into(),
            level: LogLevel::Info,
            message: "m".into(),
        }).unwrap_err();
        assert!(matches!(err, LogsError::Guard(GuardError::PersonaForbidden { .. })));
    }

    #[test]
    fn append_empty_message_rejected() {
        let s = LogStore::new();
        let err = s.append(Some(&op()), AppendLogRequest {
            tenant: "acme".into(),
            app: "web".into(),
            instance: "i".into(),
            level: LogLevel::Info,
            message: "".into(),
        }).unwrap_err();
        assert!(matches!(err, LogsError::InvalidMessage(_)));
    }

    #[test]
    fn append_huge_message_rejected() {
        let s = LogStore::new();
        let m = "x".repeat(20_000);
        let err = s.append(Some(&op()), AppendLogRequest {
            tenant: "acme".into(),
            app: "web".into(),
            instance: "i".into(),
            level: LogLevel::Info,
            message: m,
        }).unwrap_err();
        assert!(matches!(err, LogsError::InvalidMessage(_)));
    }

    #[test]
    fn append_assigns_increasing_ids() {
        let s = LogStore::new();
        let e1 = append(&s, "acme", "web", LogLevel::Info, "m1");
        let e2 = append(&s, "acme", "web", LogLevel::Info, "m2");
        assert_eq!(e2.id, e1.id + 1);
    }

    #[test]
    fn query_anonymous_denied() {
        let s = LogStore::new();
        let q = LogQuery { tenant: "acme".into(), ..Default::default() };
        let err = s.query(None, &q).unwrap_err();
        assert!(matches!(err, LogsError::Guard(GuardError::Anonymous)));
    }

    #[test]
    fn query_dev_cross_tenant_denied() {
        let s = LogStore::new();
        append(&s, "acme", "web", LogLevel::Info, "m");
        let q = LogQuery { tenant: "acme".into(), ..Default::default() };
        let err = s.query(Some(&dev("globex")), &q).unwrap_err();
        assert!(matches!(err, LogsError::Guard(GuardError::TenantMismatch { .. })));
    }

    #[test]
    fn query_filters_by_app() {
        let s = LogStore::new();
        append(&s, "acme", "web", LogLevel::Info, "m1");
        append(&s, "acme", "api", LogLevel::Info, "m2");
        let q = LogQuery {
            tenant: "acme".into(),
            app: Some("web".into()),
            ..Default::default()
        };
        let out = s.query(Some(&dev("acme")), &q).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].app, "web");
    }

    #[test]
    fn query_filters_by_min_level() {
        let s = LogStore::new();
        append(&s, "acme", "web", LogLevel::Trace, "t");
        append(&s, "acme", "web", LogLevel::Info, "i");
        append(&s, "acme", "web", LogLevel::Error, "e");
        let q = LogQuery {
            tenant: "acme".into(),
            min_level: Some(LogLevel::Warn),
            ..Default::default()
        };
        let out = s.query(Some(&dev("acme")), &q).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].level, LogLevel::Error);
    }

    #[test]
    fn query_filters_by_contains() {
        let s = LogStore::new();
        append(&s, "acme", "web", LogLevel::Info, "started server");
        append(&s, "acme", "web", LogLevel::Info, "request handled");
        let q = LogQuery {
            tenant: "acme".into(),
            contains: Some("server".into()),
            ..Default::default()
        };
        let out = s.query(Some(&dev("acme")), &q).unwrap();
        assert_eq!(out.len(), 1);
        assert!(out[0].message.contains("server"));
    }

    #[test]
    fn query_filters_by_instance() {
        let s = LogStore::new();
        s.append(Some(&op()), AppendLogRequest {
            tenant: "acme".into(),
            app: "web".into(),
            instance: "i-1".into(),
            level: LogLevel::Info,
            message: "from-1".into(),
        }).unwrap();
        s.append(Some(&op()), AppendLogRequest {
            tenant: "acme".into(),
            app: "web".into(),
            instance: "i-2".into(),
            level: LogLevel::Info,
            message: "from-2".into(),
        }).unwrap();
        let q = LogQuery {
            tenant: "acme".into(),
            instance: Some("i-2".into()),
            ..Default::default()
        };
        let out = s.query(Some(&dev("acme")), &q).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].message, "from-2");
    }

    #[test]
    fn query_since_id_returns_only_newer() {
        let s = LogStore::new();
        let e1 = append(&s, "acme", "web", LogLevel::Info, "m1");
        append(&s, "acme", "web", LogLevel::Info, "m2");
        let q = LogQuery {
            tenant: "acme".into(),
            since_id: Some(e1.id),
            ..Default::default()
        };
        let out = s.query(Some(&dev("acme")), &q).unwrap();
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn query_limit_caps_result() {
        let s = LogStore::new();
        for i in 0..500 {
            append(&s, "acme", "web", LogLevel::Info, &format!("m{i}"));
        }
        let q = LogQuery { tenant: "acme".into(), limit: Some(20), ..Default::default() };
        let out = s.query(Some(&dev("acme")), &q).unwrap();
        assert_eq!(out.len(), 20);
    }

    #[test]
    fn query_default_limit_is_200() {
        let s = LogStore::new();
        for i in 0..500 {
            append(&s, "acme", "web", LogLevel::Info, &format!("m{i}"));
        }
        let q = LogQuery { tenant: "acme".into(), ..Default::default() };
        let out = s.query(Some(&dev("acme")), &q).unwrap();
        assert_eq!(out.len(), 200);
    }

    #[test]
    fn query_limit_too_large_rejected() {
        let s = LogStore::new();
        let q = LogQuery { tenant: "acme".into(), limit: Some(10_000), ..Default::default() };
        let err = s.query(Some(&dev("acme")), &q).unwrap_err();
        assert_eq!(err, LogsError::LimitTooLarge);
    }

    #[test]
    fn query_returns_descending_by_id() {
        let s = LogStore::new();
        for i in 0..5 {
            append(&s, "acme", "web", LogLevel::Info, &format!("m{i}"));
        }
        let q = LogQuery { tenant: "acme".into(), ..Default::default() };
        let out = s.query(Some(&dev("acme")), &q).unwrap();
        let ids: Vec<u64> = out.iter().map(|e| e.id).collect();
        let mut desc = ids.clone();
        desc.sort_by(|a, b| b.cmp(a));
        assert_eq!(ids, desc);
    }

    #[test]
    fn store_count_tracks_appends() {
        let s = LogStore::new();
        append(&s, "acme", "web", LogLevel::Info, "m");
        assert_eq!(s.count(), 1);
    }
}

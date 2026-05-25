// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Events — append-only audit feed for portal-visible activity.

use std::collections::VecDeque;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::routes::rbac::{Guard, GuardError, Principal};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventLevel {
    Debug,
    Info,
    Warn,
    Error,
}

impl EventLevel {
    pub fn rank(&self) -> u8 {
        match self {
            EventLevel::Debug => 0,
            EventLevel::Info => 1,
            EventLevel::Warn => 2,
            EventLevel::Error => 3,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    pub id: u64,
    pub tenant: String,
    pub source: String,
    pub level: EventLevel,
    pub message: String,
    pub timestamp: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppendEventRequest {
    pub tenant: String,
    pub source: String,
    pub level: EventLevel,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventQuery {
    #[serde(default)]
    pub tenant: Option<String>,
    #[serde(default)]
    pub min_level: Option<EventLevel>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub since_id: Option<u64>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EventsError {
    #[error("guard: {0}")]
    Guard(#[from] GuardError),
    #[error("limit too large (max 1000)")]
    LimitTooLarge,
    #[error("invalid message: {0}")]
    InvalidMessage(String),
}

const RING_CAPACITY: usize = 10_000;
const DEFAULT_LIMIT: usize = 100;
const MAX_LIMIT: usize = 1_000;

pub struct EventStore {
    inner: Mutex<VecDeque<Event>>,
    seq: Mutex<u64>,
}

impl Default for EventStore {
    fn default() -> Self {
        Self {
            inner: Mutex::new(VecDeque::with_capacity(RING_CAPACITY)),
            seq: Mutex::new(0),
        }
    }
}

impl EventStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn append(
        &self,
        principal: Option<&Principal>,
        req: AppendEventRequest,
    ) -> Result<Event, EventsError> {
        Guard::cross_persona(None).authorize(principal, Some(&req.tenant))?;
        if req.message.is_empty() {
            return Err(EventsError::InvalidMessage("empty".into()));
        }
        if req.message.len() > 4096 {
            return Err(EventsError::InvalidMessage("too long".into()));
        }
        let mut seq = self.seq.lock().unwrap();
        *seq += 1;
        let id = *seq;
        drop(seq);
        let event = Event {
            id,
            tenant: req.tenant,
            source: req.source,
            level: req.level,
            message: req.message,
            timestamp: "1970-01-01T00:00:00Z".into(),
        };
        let mut buf = self.inner.lock().unwrap();
        if buf.len() >= RING_CAPACITY {
            buf.pop_front();
        }
        buf.push_back(event.clone());
        Ok(event)
    }

    pub fn query(
        &self,
        principal: Option<&Principal>,
        q: &EventQuery,
    ) -> Result<Vec<Event>, EventsError> {
        // For tenant queries, enforce tenant scoping; for cross-tenant queries
        // (no tenant filter), only operator/admin allowed.
        if let Some(tenant) = &q.tenant {
            Guard::cross_persona(None).authorize(principal, Some(tenant))?;
        } else {
            Guard::operator_only().authorize(principal, None)?;
        }

        let limit = q.limit.unwrap_or(DEFAULT_LIMIT);
        if limit > MAX_LIMIT {
            return Err(EventsError::LimitTooLarge);
        }

        let buf = self.inner.lock().unwrap();
        let mut out: Vec<Event> = buf
            .iter()
            .rev()
            .filter(|e| {
                if let Some(t) = &q.tenant {
                    if &e.tenant != t {
                        return false;
                    }
                }
                if let Some(min) = q.min_level {
                    if e.level.rank() < min.rank() {
                        return false;
                    }
                }
                if let Some(src) = &q.source {
                    if &e.source != src {
                        return false;
                    }
                }
                if let Some(since) = q.since_id {
                    if e.id <= since {
                        return false;
                    }
                }
                true
            })
            .cloned()
            .take(limit)
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

    fn dev(t: &str) -> Principal {
        Principal::new("d", Persona::Tenant).with_tenant(t)
    }
    fn op() -> Principal {
        Principal::new("o", Persona::Operator)
    }
    fn admin() -> Principal {
        Principal::new("a", Persona::Admin)
    }

    fn append(s: &EventStore, t: &str, src: &str, lvl: EventLevel, msg: &str) -> Event {
        s.append(
            Some(&admin()),
            AppendEventRequest {
                tenant: t.into(),
                source: src.into(),
                level: lvl,
                message: msg.into(),
            },
        )
        .unwrap()
    }

    #[test]
    fn level_rank_ordering() {
        assert!(EventLevel::Debug.rank() < EventLevel::Info.rank());
        assert!(EventLevel::Info.rank() < EventLevel::Warn.rank());
        assert!(EventLevel::Warn.rank() < EventLevel::Error.rank());
    }

    #[test]
    fn append_anonymous_denied() {
        let s = EventStore::new();
        let err = s
            .append(
                None,
                AppendEventRequest {
                    tenant: "acme".into(),
                    source: "x".into(),
                    level: EventLevel::Info,
                    message: "m".into(),
                },
            )
            .unwrap_err();
        assert!(matches!(err, EventsError::Guard(GuardError::Anonymous)));
    }

    #[test]
    fn append_empty_message_rejected() {
        let s = EventStore::new();
        let err = s
            .append(
                Some(&admin()),
                AppendEventRequest {
                    tenant: "acme".into(),
                    source: "x".into(),
                    level: EventLevel::Info,
                    message: "".into(),
                },
            )
            .unwrap_err();
        assert!(matches!(err, EventsError::InvalidMessage(_)));
    }

    #[test]
    fn append_huge_message_rejected() {
        let s = EventStore::new();
        let m = "x".repeat(5000);
        let err = s
            .append(
                Some(&admin()),
                AppendEventRequest {
                    tenant: "acme".into(),
                    source: "x".into(),
                    level: EventLevel::Info,
                    message: m,
                },
            )
            .unwrap_err();
        assert!(matches!(err, EventsError::InvalidMessage(_)));
    }

    #[test]
    fn append_assigns_increasing_ids() {
        let s = EventStore::new();
        let e1 = append(&s, "acme", "src", EventLevel::Info, "m1");
        let e2 = append(&s, "acme", "src", EventLevel::Info, "m2");
        assert_eq!(e2.id, e1.id + 1);
    }

    #[test]
    fn query_returns_all_for_tenant() {
        let s = EventStore::new();
        append(&s, "acme", "x", EventLevel::Info, "m1");
        append(&s, "globex", "x", EventLevel::Info, "m2");
        let q = EventQuery {
            tenant: Some("acme".into()),
            ..Default::default()
        };
        let out = s.query(Some(&dev("acme")), &q).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].tenant, "acme");
    }

    #[test]
    fn query_filters_by_min_level() {
        let s = EventStore::new();
        append(&s, "acme", "x", EventLevel::Debug, "d");
        append(&s, "acme", "x", EventLevel::Info, "i");
        append(&s, "acme", "x", EventLevel::Warn, "w");
        append(&s, "acme", "x", EventLevel::Error, "e");
        let q = EventQuery {
            tenant: Some("acme".into()),
            min_level: Some(EventLevel::Warn),
            ..Default::default()
        };
        let out = s.query(Some(&dev("acme")), &q).unwrap();
        assert_eq!(out.len(), 2);
        assert!(
            out.iter()
                .all(|e| e.level.rank() >= EventLevel::Warn.rank())
        );
    }

    #[test]
    fn query_filters_by_source() {
        let s = EventStore::new();
        append(&s, "acme", "src-a", EventLevel::Info, "1");
        append(&s, "acme", "src-b", EventLevel::Info, "2");
        let q = EventQuery {
            tenant: Some("acme".into()),
            source: Some("src-a".into()),
            ..Default::default()
        };
        let out = s.query(Some(&dev("acme")), &q).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].source, "src-a");
    }

    #[test]
    fn query_since_id_returns_only_newer() {
        let s = EventStore::new();
        let e1 = append(&s, "acme", "x", EventLevel::Info, "1");
        append(&s, "acme", "x", EventLevel::Info, "2");
        append(&s, "acme", "x", EventLevel::Info, "3");
        let q = EventQuery {
            tenant: Some("acme".into()),
            since_id: Some(e1.id),
            ..Default::default()
        };
        let out = s.query(Some(&dev("acme")), &q).unwrap();
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn query_limit_caps_result() {
        let s = EventStore::new();
        for i in 0..200 {
            append(&s, "acme", "x", EventLevel::Info, &format!("m{i}"));
        }
        let q = EventQuery {
            tenant: Some("acme".into()),
            limit: Some(10),
            ..Default::default()
        };
        let out = s.query(Some(&dev("acme")), &q).unwrap();
        assert_eq!(out.len(), 10);
    }

    #[test]
    fn query_default_limit_is_100() {
        let s = EventStore::new();
        for i in 0..200 {
            append(&s, "acme", "x", EventLevel::Info, &format!("m{i}"));
        }
        let q = EventQuery {
            tenant: Some("acme".into()),
            ..Default::default()
        };
        let out = s.query(Some(&dev("acme")), &q).unwrap();
        assert_eq!(out.len(), 100);
    }

    #[test]
    fn query_limit_too_large_rejected() {
        let s = EventStore::new();
        let q = EventQuery {
            tenant: Some("acme".into()),
            limit: Some(2000),
            ..Default::default()
        };
        let err = s.query(Some(&dev("acme")), &q).unwrap_err();
        assert_eq!(err, EventsError::LimitTooLarge);
    }

    #[test]
    fn query_returns_descending_by_id() {
        let s = EventStore::new();
        for i in 0..5 {
            append(&s, "acme", "x", EventLevel::Info, &format!("m{i}"));
        }
        let q = EventQuery {
            tenant: Some("acme".into()),
            ..Default::default()
        };
        let out = s.query(Some(&dev("acme")), &q).unwrap();
        let ids: Vec<u64> = out.iter().map(|e| e.id).collect();
        let mut sorted_desc = ids.clone();
        sorted_desc.sort_by(|a, b| b.cmp(a));
        assert_eq!(ids, sorted_desc);
    }

    #[test]
    fn query_cross_tenant_requires_operator() {
        let s = EventStore::new();
        append(&s, "acme", "x", EventLevel::Info, "m");
        let q = EventQuery::default();
        let err = s.query(Some(&dev("acme")), &q).unwrap_err();
        assert!(matches!(
            err,
            EventsError::Guard(GuardError::PersonaForbidden { .. })
        ));
    }

    #[test]
    fn query_cross_tenant_allowed_for_operator() {
        let s = EventStore::new();
        append(&s, "acme", "x", EventLevel::Info, "m");
        append(&s, "globex", "x", EventLevel::Info, "m");
        let q = EventQuery::default();
        let out = s.query(Some(&op()), &q).unwrap();
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn query_dev_cross_tenant_denied() {
        let s = EventStore::new();
        append(&s, "globex", "x", EventLevel::Info, "m");
        let q = EventQuery {
            tenant: Some("globex".into()),
            ..Default::default()
        };
        let err = s.query(Some(&dev("acme")), &q).unwrap_err();
        assert!(matches!(
            err,
            EventsError::Guard(GuardError::TenantMismatch { .. })
        ));
    }

    #[test]
    fn store_count_tracks_appends() {
        let s = EventStore::new();
        assert_eq!(s.count(), 0);
        append(&s, "acme", "x", EventLevel::Info, "m");
        assert_eq!(s.count(), 1);
    }

    #[test]
    fn ring_caps_at_capacity() {
        let s = EventStore::new();
        for i in 0..(RING_CAPACITY + 50) {
            append(&s, "acme", "x", EventLevel::Info, &format!("m{i}"));
        }
        assert_eq!(s.count(), RING_CAPACITY);
    }
}

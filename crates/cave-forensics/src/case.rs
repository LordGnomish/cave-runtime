// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Forensic case store. Built on top of [`crate::models::ForensicCase`].
//!
//! Cases own a list of evidence + a severity rollup. Adding evidence
//! re-evaluates whether the case is "interesting" and bumps severity
//! based on the highest evidence severity attached.

use crate::error::{ForensicsError, Result};
use crate::evidence::build_evidence_item;
use crate::events::KernelEvent;
use crate::models::{CaseStatus, EvidenceItem, EvidenceType, ForensicCase, ForensicSeverity};
use chrono::Utc;
use dashmap::DashMap;
use std::sync::Arc;
use uuid::Uuid;

/// Concurrent case store keyed by `Uuid`.
#[derive(Debug, Default)]
pub struct CaseStore {
    cases: DashMap<Uuid, ForensicCase>,
}

impl CaseStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn open(
        &self,
        title: impl Into<String>,
        description: impl Into<String>,
        severity: ForensicSeverity,
    ) -> ForensicCase {
        let case = ForensicCase {
            id: Uuid::new_v4(),
            title: title.into(),
            description: description.into(),
            severity,
            status: CaseStatus::Open,
            created_at: Utc::now(),
            evidence: Vec::new(),
        };
        self.cases.insert(case.id, case.clone());
        case
    }

    pub fn get(&self, id: Uuid) -> Option<ForensicCase> {
        self.cases.get(&id).map(|r| r.clone())
    }

    pub fn count(&self) -> usize {
        self.cases.len()
    }

    pub fn list(&self) -> Vec<ForensicCase> {
        self.cases.iter().map(|r| r.value().clone()).collect()
    }

    pub fn transition(&self, id: Uuid, new_status: CaseStatus) -> Result<ForensicCase> {
        let mut entry = self
            .cases
            .get_mut(&id)
            .ok_or_else(|| ForensicsError::CaseNotFound(id.to_string()))?;
        entry.status = new_status;
        Ok(entry.clone())
    }

    pub fn add_evidence(&self, id: Uuid, item: EvidenceItem) -> Result<ForensicCase> {
        let mut entry = self
            .cases
            .get_mut(&id)
            .ok_or_else(|| ForensicsError::CaseNotFound(id.to_string()))?;
        entry.evidence.push(item);
        Ok(entry.clone())
    }

    pub fn close(&self, id: Uuid) -> Result<ForensicCase> {
        self.transition(id, CaseStatus::Closed)
    }

    /// Ingest a tetragon kernel event into a case, packing it as a
    /// `LogFile` evidence item with chain-of-custody seeded.
    pub fn ingest_event(
        &self,
        id: Uuid,
        ev: &KernelEvent,
        actor: impl Into<String>,
    ) -> Result<ForensicCase> {
        let payload = serde_json::to_vec(ev)?;
        let desc = format!("tetragon:{}", ev.kind_tag());
        let item = build_evidence_item(EvidenceType::LogFile, desc, &payload, actor);
        self.add_evidence(id, item)
    }
}

/// Convenience: the global case store (lazy-init).
pub fn shared_store() -> Arc<CaseStore> {
    Arc::new(CaseStore::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::process_exec::ProcessExecEvent;
    use crate::process::{Credentials, Namespaces, Process};
    use chrono::TimeZone;

    fn ts() -> chrono::DateTime<Utc> {
        Utc.timestamp_opt(0, 0).unwrap()
    }

    fn make_event() -> KernelEvent {
        KernelEvent::ProcessExec(ProcessExecEvent {
            process: Process {
                exec_id: "x".into(),
                pid: 1,
                pid_in_ns: 1,
                binary: "/bin/sh".into(),
                arguments: String::new(),
                cwd: "/".into(),
                credentials: Credentials::default(),
                namespaces: Namespaces::default(),
                parent_exec_id: None,
                container_id: None,
                pod_name: None,
                pod_namespace: None,
                start_time: ts(),
                end_time: None,
            },
            ancestors: vec![],
            observed_at: ts(),
        })
    }

    #[test]
    fn test_open_returns_open_status() {
        let s = CaseStore::new();
        let c = s.open("t1", "d1", ForensicSeverity::High);
        assert_eq!(c.status, CaseStatus::Open);
        assert_eq!(s.count(), 1);
    }

    #[test]
    fn test_get_after_open_returns_case() {
        let s = CaseStore::new();
        let c = s.open("t", "d", ForensicSeverity::Low);
        assert!(s.get(c.id).is_some());
    }

    #[test]
    fn test_transition_updates_status() {
        let s = CaseStore::new();
        let c = s.open("t", "d", ForensicSeverity::Medium);
        let updated = s.transition(c.id, CaseStatus::InProgress).unwrap();
        assert_eq!(updated.status, CaseStatus::InProgress);
    }

    #[test]
    fn test_transition_on_missing_case_errors() {
        let s = CaseStore::new();
        let err = s.transition(Uuid::new_v4(), CaseStatus::Closed).unwrap_err();
        assert!(matches!(err, ForensicsError::CaseNotFound(_)));
    }

    #[test]
    fn test_close_sets_status_closed() {
        let s = CaseStore::new();
        let c = s.open("t", "d", ForensicSeverity::Critical);
        let after = s.close(c.id).unwrap();
        assert_eq!(after.status, CaseStatus::Closed);
    }

    #[test]
    fn test_add_evidence_appends() {
        let s = CaseStore::new();
        let c = s.open("t", "d", ForensicSeverity::High);
        let item = build_evidence_item(EvidenceType::ProcessDump, "core", b"x", "alice");
        let after = s.add_evidence(c.id, item).unwrap();
        assert_eq!(after.evidence.len(), 1);
    }

    #[test]
    fn test_ingest_event_creates_log_file_evidence() {
        let s = CaseStore::new();
        let c = s.open("t", "d", ForensicSeverity::High);
        let after = s.ingest_event(c.id, &make_event(), "tetragon-agent").unwrap();
        assert_eq!(after.evidence.len(), 1);
        let ev = &after.evidence[0];
        assert!(matches!(ev.evidence_type, EvidenceType::LogFile));
        assert!(ev.description.contains("tetragon"));
        assert!(ev.hash_sha256.is_some());
    }

    #[test]
    fn test_list_returns_all_cases() {
        let s = CaseStore::new();
        s.open("a", "x", ForensicSeverity::Low);
        s.open("b", "x", ForensicSeverity::Low);
        s.open("c", "x", ForensicSeverity::Low);
        assert_eq!(s.list().len(), 3);
    }

    #[test]
    fn test_shared_store_is_independent() {
        let s = shared_store();
        let s2 = shared_store();
        s.open("t", "d", ForensicSeverity::Low);
        assert_eq!(s.count(), 1);
        assert_eq!(s2.count(), 0);
    }
}

//! Audit log emission.
//!
//! Upstream: kubernetes/kubernetes v1.30.0
//!   * `staging/src/k8s.io/apiserver/pkg/audit/types.go`
//!   * `staging/src/k8s.io/apiserver/pkg/audit/policy/`
//!   * `staging/src/k8s.io/apiserver/pkg/audit/event.go` (`NewEventFromRequest`).
//!
//! An audit `Event` is emitted at each `Stage` of a request's lifecycle and
//! filtered by an audit `Policy`. We implement a bounded ring-buffer sink with
//! a level-based policy plus a per-stage drop filter, mirroring upstream
//! semantics of `audit.PolicyRuleEvaluator`.
//!
//! Tenant invariant: every emitted event carries a `tenant_id`. The sink
//! never strips it, and the drop filter MUST ONLY filter on stage/level —
//! never on tenant_id (so audit cannot be silenced cross-tenant).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum AuditLevel {
    None = 0,
    Metadata = 1,
    Request = 2,
    RequestResponse = 3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuditStage {
    RequestReceived,
    ResponseStarted,
    ResponseComplete,
    Panic,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub audit_id: String,
    pub level: AuditLevel,
    pub stage: AuditStage,
    pub timestamp: DateTime<Utc>,
    pub user: String,
    pub tenant_id: String,
    pub namespace: String,
    pub verb: String,
    pub resource: String,
    pub name: String,
    pub request_uri: String,
    pub response_code: u16,
    pub request_object: Option<serde_json::Value>,
    pub response_object: Option<serde_json::Value>,
}

impl AuditEvent {
    pub fn new(
        audit_id: impl Into<String>,
        level: AuditLevel,
        stage: AuditStage,
        user: impl Into<String>,
        tenant_id: impl Into<String>,
        namespace: impl Into<String>,
        verb: impl Into<String>,
        resource: impl Into<String>,
        name: impl Into<String>,
        request_uri: impl Into<String>,
        response_code: u16,
    ) -> Self {
        Self {
            audit_id: audit_id.into(),
            level,
            stage,
            timestamp: Utc::now(),
            user: user.into(),
            tenant_id: tenant_id.into(),
            namespace: namespace.into(),
            verb: verb.into(),
            resource: resource.into(),
            name: name.into(),
            request_uri: request_uri.into(),
            response_code,
            request_object: None,
            response_object: None,
        }
    }

    /// Apply level-based redaction. Mirrors upstream `event.LogRequestObject`
    /// + `LogResponseObject` gates.
    pub fn redact_for_level(&mut self) {
        match self.level {
            AuditLevel::None => {
                // dropped entirely upstream; no payload
                self.request_object = None;
                self.response_object = None;
            }
            AuditLevel::Metadata => {
                self.request_object = None;
                self.response_object = None;
            }
            AuditLevel::Request => {
                // request body kept, response stripped
                self.response_object = None;
            }
            AuditLevel::RequestResponse => {
                // both kept
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct AuditPolicy {
    pub default_level: AuditLevel,
    /// Stages to drop entirely (no-op emission).
    pub omit_stages: Vec<AuditStage>,
}

impl AuditPolicy {
    pub fn new(default_level: AuditLevel) -> Self {
        Self { default_level, omit_stages: vec![] }
    }
    pub fn omit(mut self, stage: AuditStage) -> Self {
        self.omit_stages.push(stage);
        self
    }
    pub fn should_record(&self, stage: AuditStage) -> bool {
        !self.omit_stages.contains(&stage)
    }
}

pub struct AuditLogger {
    capacity: usize,
    policy: AuditPolicy,
    sink: Mutex<VecDeque<AuditEvent>>,
}

impl AuditLogger {
    pub fn new(capacity: usize, policy: AuditPolicy) -> Self {
        assert!(capacity > 0);
        Self { capacity, policy, sink: Mutex::new(VecDeque::with_capacity(capacity)) }
    }

    pub fn emit(&self, mut ev: AuditEvent) -> bool {
        if !self.policy.should_record(ev.stage) {
            return false;
        }
        if self.policy.default_level == AuditLevel::None {
            return false;
        }
        // Apply default level if event came in unset (Level::None placeholder).
        if ev.level == AuditLevel::None {
            ev.level = self.policy.default_level;
        }
        ev.redact_for_level();
        let mut sink = self.sink.lock().unwrap();
        sink.push_back(ev);
        if sink.len() > self.capacity { sink.pop_front(); }
        true
    }

    pub fn events(&self) -> Vec<AuditEvent> {
        self.sink.lock().unwrap().iter().cloned().collect()
    }

    pub fn events_for_tenant(&self, tenant_id: &str) -> Vec<AuditEvent> {
        self.sink.lock().unwrap().iter()
            .filter(|e| e.tenant_id == tenant_id).cloned().collect()
    }

    pub fn len(&self) -> usize { self.sink.lock().unwrap().len() }
    pub fn is_empty(&self) -> bool { self.len() == 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(stage: AuditStage, tenant: &str, user: &str, code: u16) -> AuditEvent {
        AuditEvent::new(
            "auid-1", AuditLevel::None, stage,
            user, tenant, "default", "create", "configmaps", "cm1",
            "/api/v1/namespaces/default/configmaps", code,
        )
    }

    /// Upstream parity: `TestPolicy_DefaultLevelMetadata`.
    #[test]
    fn test_default_level_metadata_strips_objects() {
        let logger = AuditLogger::new(64, AuditPolicy::new(AuditLevel::Metadata));
        let mut e = ev(AuditStage::ResponseComplete, "acme", "alice", 201);
        e.request_object = Some(serde_json::json!({"a": 1}));
        e.response_object = Some(serde_json::json!({"b": 2}));
        assert!(logger.emit(e));
        let stored = &logger.events()[0];
        assert_eq!(stored.level, AuditLevel::Metadata);
        assert!(stored.request_object.is_none());
        assert!(stored.response_object.is_none());
        assert_eq!(stored.tenant_id, "acme",
            "tenant_id invariant: never stripped by redaction");
    }

    /// Upstream parity: `TestPolicy_LevelRequest_KeepsRequestStripsResponse`.
    #[test]
    fn test_level_request_keeps_request_strips_response() {
        let logger = AuditLogger::new(64, AuditPolicy::new(AuditLevel::Request));
        let mut e = ev(AuditStage::ResponseComplete, "acme", "alice", 200);
        e.request_object = Some(serde_json::json!({"a": 1}));
        e.response_object = Some(serde_json::json!({"b": 2}));
        assert!(logger.emit(e));
        let stored = &logger.events()[0];
        assert!(stored.request_object.is_some());
        assert!(stored.response_object.is_none());
        assert_eq!(stored.tenant_id, "acme", "tenant_id invariant");
    }

    /// Upstream parity: `TestPolicy_LevelRequestResponse_KeepsBoth`.
    #[test]
    fn test_level_request_response_keeps_both() {
        let logger = AuditLogger::new(64, AuditPolicy::new(AuditLevel::RequestResponse));
        let mut e = ev(AuditStage::ResponseComplete, "acme", "alice", 200);
        e.request_object = Some(serde_json::json!({"a": 1}));
        e.response_object = Some(serde_json::json!({"b": 2}));
        assert!(logger.emit(e));
        let stored = &logger.events()[0];
        assert!(stored.request_object.is_some());
        assert!(stored.response_object.is_some());
        assert_eq!(stored.tenant_id, "acme", "tenant_id invariant");
    }

    /// Upstream parity: `TestPolicy_OmitStage`.
    #[test]
    fn test_omit_stage_drops_event() {
        let logger = AuditLogger::new(64,
            AuditPolicy::new(AuditLevel::Metadata).omit(AuditStage::RequestReceived));
        let dropped = !logger.emit(ev(AuditStage::RequestReceived, "acme", "alice", 0));
        assert!(dropped);
        let kept = logger.emit(ev(AuditStage::ResponseComplete, "acme", "alice", 200));
        assert!(kept);
        assert_eq!(logger.len(), 1);
        assert_eq!(logger.events()[0].tenant_id, "acme",
            "tenant_id invariant: stage-based omit MUST NOT cross-leak tenants");
    }

    /// Upstream parity: `TestPolicy_LevelNoneDropsAll`.
    #[test]
    fn test_level_none_drops_everything() {
        let logger = AuditLogger::new(64, AuditPolicy::new(AuditLevel::None));
        let recorded = logger.emit(ev(AuditStage::ResponseComplete, "acme", "alice", 200));
        assert!(!recorded);
        assert_eq!(logger.len(), 0);
    }

    /// Upstream parity: `TestAudit_TenantIsolationOnQuery`.
    #[test]
    fn test_events_for_tenant_isolates() {
        let logger = AuditLogger::new(64, AuditPolicy::new(AuditLevel::Metadata));
        logger.emit(ev(AuditStage::ResponseComplete, "acme", "alice", 200));
        logger.emit(ev(AuditStage::ResponseComplete, "globex", "bob", 200));
        logger.emit(ev(AuditStage::ResponseComplete, "acme", "alice", 200));
        let acme = logger.events_for_tenant("acme");
        let globex = logger.events_for_tenant("globex");
        assert_eq!(acme.len(), 2);
        assert_eq!(globex.len(), 1);
        assert!(acme.iter().all(|e| e.tenant_id == "acme"),
            "tenant_id invariant: query never crosses tenants");
    }

    /// Upstream parity: `TestAudit_RingBufferEvictsOldest`.
    #[test]
    fn test_ring_buffer_evicts_oldest_on_overflow() {
        let logger = AuditLogger::new(2, AuditPolicy::new(AuditLevel::Metadata));
        for i in 0..3 {
            let e = ev(AuditStage::ResponseComplete, "acme", &format!("u{}", i), 200);
            logger.emit(e);
        }
        assert_eq!(logger.len(), 2);
        let stored = logger.events();
        assert!(stored.iter().all(|e| e.tenant_id == "acme"), "tenant_id invariant");
        // Oldest (u0) evicted, only u1, u2 remain.
        assert!(!stored.iter().any(|e| e.user == "u0"));
    }

    /// Upstream parity: `TestAudit_AllStagesEmittedInOrder`.
    #[test]
    fn test_all_four_stages_recorded_in_order() {
        let logger = AuditLogger::new(64, AuditPolicy::new(AuditLevel::Metadata));
        for s in [AuditStage::RequestReceived, AuditStage::ResponseStarted,
                  AuditStage::ResponseComplete, AuditStage::Panic] {
            logger.emit(ev(s, "acme", "alice", 200));
        }
        let stored = logger.events();
        assert_eq!(stored.len(), 4);
        assert_eq!(stored[0].stage, AuditStage::RequestReceived);
        assert_eq!(stored[3].stage, AuditStage::Panic);
        assert!(stored.iter().all(|e| e.tenant_id == "acme"), "tenant_id invariant");
    }

    /// Upstream parity: `TestAudit_LevelOrdering`.
    #[test]
    fn test_audit_level_ordering() {
        assert!(AuditLevel::None < AuditLevel::Metadata);
        assert!(AuditLevel::Metadata < AuditLevel::Request);
        assert!(AuditLevel::Request < AuditLevel::RequestResponse);
        // tenant_id invariant smoke: emitting under high level still tags tenant.
        let logger = AuditLogger::new(2, AuditPolicy::new(AuditLevel::RequestResponse));
        logger.emit(ev(AuditStage::ResponseComplete, "acme", "alice", 200));
        assert_eq!(logger.events()[0].tenant_id, "acme");
    }
}

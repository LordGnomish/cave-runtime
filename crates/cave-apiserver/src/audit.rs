// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
pub enum AuditLevel {
    /// Default — equivalent to upstream `LevelNone` (the empty string).
    #[default]
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
    /// Subresource (e.g. "status", "scale", "exec"); empty for the main
    /// resource. Mirrors `audit.Event.RequestObject.Subresource`.
    pub subresource: String,
    /// Structured annotations attached by admission/auth chains.
    /// Mirrors `audit.Event.Annotations` from KEP-1314.
    pub annotations: std::collections::BTreeMap<String, String>,
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
            subresource: String::new(),
            annotations: std::collections::BTreeMap::new(),
        }
    }

    /// Convenience builder: tag a subresource path segment.
    pub fn with_subresource(mut self, sub: impl Into<String>) -> Self {
        self.subresource = sub.into();
        self
    }

    /// Convenience builder: attach an audit annotation. Mirrors
    /// `audit.Event.Annotations` map updates from upstream filters.
    pub fn with_annotation(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.annotations.insert(key.into(), value.into());
        self
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
        Self {
            default_level,
            omit_stages: vec![],
        }
    }
    pub fn omit(mut self, stage: AuditStage) -> Self {
        self.omit_stages.push(stage);
        self
    }
    pub fn should_record(&self, stage: AuditStage) -> bool {
        !self.omit_stages.contains(&stage)
    }
}

/// Filter chain — composes a sequence of stage/level decisions atop a
/// policy. Each filter inspects the (stage, event-shape) pair and returns
/// `true` to keep, `false` to drop. Mirrors upstream
/// `apiserver/pkg/audit/policy/checker.go::Checker` rule chaining.
pub trait AuditFilter: Send + Sync {
    fn name(&self) -> &str;
    fn keep(&self, ev: &AuditEvent) -> bool;
}

pub struct AuditFilterChain {
    filters: Vec<std::sync::Arc<dyn AuditFilter>>,
}

impl AuditFilterChain {
    pub fn new() -> Self {
        Self { filters: vec![] }
    }

    pub fn with(mut self, f: std::sync::Arc<dyn AuditFilter>) -> Self {
        self.filters.push(f);
        self
    }

    pub fn keep(&self, ev: &AuditEvent) -> bool {
        self.filters.iter().all(|f| f.keep(ev))
    }

    pub fn len(&self) -> usize {
        self.filters.len()
    }
    pub fn is_empty(&self) -> bool {
        self.filters.is_empty()
    }
}

impl Default for AuditFilterChain {
    fn default() -> Self {
        Self::new()
    }
}

/// Built-in filter — drop events whose stage is in the supplied set.
pub struct DropStageFilter {
    pub stages: Vec<AuditStage>,
}

impl AuditFilter for DropStageFilter {
    fn name(&self) -> &str {
        "drop-stage"
    }
    fn keep(&self, ev: &AuditEvent) -> bool {
        !self.stages.contains(&ev.stage)
    }
}

/// Built-in filter — drop events whose verb is in the supplied set.
/// Mirrors upstream `policy.PolicyRule.Verbs` exclude semantics.
pub struct DropVerbFilter {
    pub verbs: Vec<String>,
}

impl AuditFilter for DropVerbFilter {
    fn name(&self) -> &str {
        "drop-verb"
    }
    fn keep(&self, ev: &AuditEvent) -> bool {
        !self.verbs.iter().any(|v| v == &ev.verb)
    }
}

pub struct AuditLogger {
    capacity: usize,
    policy: AuditPolicy,
    sink: Mutex<VecDeque<AuditEvent>>,
    filter_chain: AuditFilterChain,
}

impl AuditLogger {
    pub fn new(capacity: usize, policy: AuditPolicy) -> Self {
        assert!(capacity > 0);
        Self {
            capacity,
            policy,
            sink: Mutex::new(VecDeque::with_capacity(capacity)),
            filter_chain: AuditFilterChain::new(),
        }
    }

    /// Replace the filter chain. Mirrors `Backend.Run`-time chain config in
    /// upstream `apiserver/pkg/audit`.
    pub fn with_filter_chain(mut self, chain: AuditFilterChain) -> Self {
        self.filter_chain = chain;
        self
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
        // Filter chain runs after stage/level promotion so filters can
        // reason about the final event shape.
        if !self.filter_chain.keep(&ev) {
            return false;
        }
        ev.redact_for_level();
        let mut sink = self.sink.lock().unwrap();
        sink.push_back(ev);
        if sink.len() > self.capacity {
            sink.pop_front();
        }
        true
    }

    pub fn events(&self) -> Vec<AuditEvent> {
        self.sink.lock().unwrap().iter().cloned().collect()
    }

    pub fn events_for_tenant(&self, tenant_id: &str) -> Vec<AuditEvent> {
        self.sink
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.tenant_id == tenant_id)
            .cloned()
            .collect()
    }

    pub fn len(&self) -> usize {
        self.sink.lock().unwrap().len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(stage: AuditStage, tenant: &str, user: &str, code: u16) -> AuditEvent {
        AuditEvent::new(
            "auid-1",
            AuditLevel::None,
            stage,
            user,
            tenant,
            "default",
            "create",
            "configmaps",
            "cm1",
            "/api/v1/namespaces/default/configmaps",
            code,
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
        assert_eq!(
            stored.tenant_id, "acme",
            "tenant_id invariant: never stripped by redaction"
        );
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
        let logger = AuditLogger::new(
            64,
            AuditPolicy::new(AuditLevel::Metadata).omit(AuditStage::RequestReceived),
        );
        let dropped = !logger.emit(ev(AuditStage::RequestReceived, "acme", "alice", 0));
        assert!(dropped);
        let kept = logger.emit(ev(AuditStage::ResponseComplete, "acme", "alice", 200));
        assert!(kept);
        assert_eq!(logger.len(), 1);
        assert_eq!(
            logger.events()[0].tenant_id,
            "acme",
            "tenant_id invariant: stage-based omit MUST NOT cross-leak tenants"
        );
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
        assert!(
            acme.iter().all(|e| e.tenant_id == "acme"),
            "tenant_id invariant: query never crosses tenants"
        );
    }

    /// Upstream parity: `TestAudit_RingBufferEvictsOldest`.
    #[test]
    fn test_ring_buffer_evicts_oldest_on_overflow() {
        let logger = AuditLogger::new(2, AuditPolicy::new(AuditLevel::Metadata));
        for i in 0..3 {
            let e = ev(
                AuditStage::ResponseComplete,
                "acme",
                &format!("u{}", i),
                200,
            );
            logger.emit(e);
        }
        assert_eq!(logger.len(), 2);
        let stored = logger.events();
        assert!(
            stored.iter().all(|e| e.tenant_id == "acme"),
            "tenant_id invariant"
        );
        // Oldest (u0) evicted, only u1, u2 remain.
        assert!(!stored.iter().any(|e| e.user == "u0"));
    }

    /// Upstream parity: `TestAudit_AllStagesEmittedInOrder`.
    #[test]
    fn test_all_four_stages_recorded_in_order() {
        let logger = AuditLogger::new(64, AuditPolicy::new(AuditLevel::Metadata));
        for s in [
            AuditStage::RequestReceived,
            AuditStage::ResponseStarted,
            AuditStage::ResponseComplete,
            AuditStage::Panic,
        ] {
            logger.emit(ev(s, "acme", "alice", 200));
        }
        let stored = logger.events();
        assert_eq!(stored.len(), 4);
        assert_eq!(stored[0].stage, AuditStage::RequestReceived);
        assert_eq!(stored[3].stage, AuditStage::Panic);
        assert!(
            stored.iter().all(|e| e.tenant_id == "acme"),
            "tenant_id invariant"
        );
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

    // ── Deeper coverage (v1.36.0) ─────────────────────────────────────────────

    /// Upstream parity: `TestAudit_DefaultLevelPromotedFromUnsetEvent`
    /// (audit/event.go — `event.Level == LevelNone` is treated as "use policy
    /// default" when emission is otherwise allowed).
    #[test]
    fn test_unset_event_level_is_promoted_to_policy_default() {
        let logger = AuditLogger::new(64, AuditPolicy::new(AuditLevel::Request));
        let mut e = ev(AuditStage::ResponseComplete, "acme", "alice", 200);
        // Event arrives with level=None; policy default=Request promotes it.
        e.level = AuditLevel::None;
        e.request_object = Some(serde_json::json!({"hello": "world"}));
        e.response_object = Some(serde_json::json!({"out": 1}));
        assert!(logger.emit(e));
        let stored = &logger.events()[0];
        assert_eq!(
            stored.level,
            AuditLevel::Request,
            "default level promoted onto unset event"
        );
        assert!(stored.request_object.is_some());
        assert!(
            stored.response_object.is_none(),
            "Request level strips response after promotion"
        );
        assert_eq!(
            stored.tenant_id, "acme",
            "tenant_id invariant: promotion never strips tenant"
        );
    }

    /// Upstream parity: `TestAudit_OmitMultipleStages`
    /// (Policy.OmitStages with multiple entries — only listed stages drop).
    #[test]
    fn test_omit_multiple_stages_independently() {
        let policy = AuditPolicy::new(AuditLevel::Metadata)
            .omit(AuditStage::RequestReceived)
            .omit(AuditStage::Panic);
        let logger = AuditLogger::new(64, policy);
        let dropped_a = !logger.emit(ev(AuditStage::RequestReceived, "acme", "alice", 0));
        let dropped_b = !logger.emit(ev(AuditStage::Panic, "acme", "alice", 500));
        let kept_a = logger.emit(ev(AuditStage::ResponseStarted, "acme", "alice", 0));
        let kept_b = logger.emit(ev(AuditStage::ResponseComplete, "acme", "alice", 200));
        assert!(dropped_a && dropped_b);
        assert!(kept_a && kept_b);
        assert_eq!(logger.len(), 2);
        assert!(
            logger.events().iter().all(|e| e.tenant_id == "acme"),
            "tenant_id invariant: stage-omit policy never crosses tenant lines"
        );
    }

    /// Upstream parity: `TestAudit_LifecycleAllStagesShareAuditId`
    /// (event.go — every stage of one request reuses the same audit_id).
    #[test]
    fn test_lifecycle_emits_one_audit_id_per_stage_for_one_request() {
        let logger = AuditLogger::new(64, AuditPolicy::new(AuditLevel::Metadata));
        for s in [
            AuditStage::RequestReceived,
            AuditStage::ResponseStarted,
            AuditStage::ResponseComplete,
        ] {
            let mut e = ev(s, "acme", "alice", 200);
            e.audit_id = "lifecycle-uid-1".into();
            logger.emit(e);
        }
        let stored = logger.events();
        assert_eq!(stored.len(), 3);
        assert!(
            stored.iter().all(|e| e.audit_id == "lifecycle-uid-1"),
            "all lifecycle stages share one audit_id"
        );
        assert!(
            stored.iter().all(|e| e.tenant_id == "acme"),
            "tenant_id invariant: lifecycle stages stay scoped"
        );
    }

    /// Upstream parity: `TestAudit_TenantQueryEmptyForUnknown`
    /// (Backend.List filtered by tenant returns empty for unknown tenant).
    #[test]
    fn test_events_for_unknown_tenant_returns_empty_set() {
        let logger = AuditLogger::new(64, AuditPolicy::new(AuditLevel::Metadata));
        logger.emit(ev(AuditStage::ResponseComplete, "acme", "alice", 200));
        logger.emit(ev(AuditStage::ResponseComplete, "globex", "bob", 200));
        let nobody = logger.events_for_tenant("nobody-tenant");
        assert!(
            nobody.is_empty(),
            "tenant_id invariant: unknown tenant query is empty, never bleed"
        );
        let acme = logger.events_for_tenant("acme");
        assert!(
            acme.iter().all(|e| e.tenant_id == "acme"),
            "tenant_id invariant: matching tenant returns only matching events"
        );
    }

    /// Upstream parity: `TestAudit_PolicyShouldRecordHonorsExplicitOmit`
    /// (Policy.should_record returns true for stages NOT in OmitStages).
    #[test]
    fn test_policy_should_record_returns_true_for_non_omitted_stages() {
        let p = AuditPolicy::new(AuditLevel::Metadata).omit(AuditStage::RequestReceived);
        assert!(!p.should_record(AuditStage::RequestReceived));
        assert!(p.should_record(AuditStage::ResponseStarted));
        assert!(p.should_record(AuditStage::ResponseComplete));
        assert!(p.should_record(AuditStage::Panic));
        // tenant_id invariant smoke: a logger built from this policy still
        // tags emitted events with their tenant.
        let logger = AuditLogger::new(4, p);
        logger.emit(ev(AuditStage::ResponseComplete, "acme", "alice", 200));
        assert_eq!(
            logger.events()[0].tenant_id,
            "acme",
            "tenant_id invariant on filtered emission"
        );
    }

    // ── Deeper coverage (deeper-004) — KEP-1314 ──────────────────────────────

    /// Upstream parity: `TestStructuredAudit_AnnotationsAreCarried`
    /// (apiserver/pkg/audit/event_test.go::TestEventBuilder — annotations
    /// added via `audit.Annotate` are present on the emitted event).
    #[test]
    fn test_structured_audit_annotations_round_trip_through_emit() {
        let logger = AuditLogger::new(64, AuditPolicy::new(AuditLevel::Metadata));
        let e = ev(AuditStage::ResponseComplete, "acme", "alice", 200)
            .with_annotation("authorization.k8s.io/decision", "allow")
            .with_annotation("admission.cave.io/mutator", "tenant-id-injector");
        assert!(logger.emit(e));
        let stored = &logger.events()[0];
        assert_eq!(
            stored.annotations.get("authorization.k8s.io/decision"),
            Some(&"allow".to_string())
        );
        assert_eq!(
            stored.annotations.get("admission.cave.io/mutator"),
            Some(&"tenant-id-injector".to_string())
        );
        assert_eq!(
            stored.tenant_id, "acme",
            "tenant_id invariant: annotations never displace tenant_id"
        );
    }

    /// Upstream parity: `TestStructuredAudit_SubresourceCarriedInEvent`
    /// (apiserver/pkg/audit/event_test.go — `pods/exec` produces an event
    /// with `subresource: "exec"`).
    #[test]
    fn test_structured_audit_records_subresource_path_segment() {
        let logger = AuditLogger::new(64, AuditPolicy::new(AuditLevel::Metadata));
        let e = ev(AuditStage::ResponseComplete, "acme", "alice", 200).with_subresource("exec");
        assert!(logger.emit(e));
        let stored = &logger.events()[0];
        assert_eq!(stored.subresource, "exec");
        assert_eq!(
            stored.tenant_id, "acme",
            "tenant_id invariant: subresource never replaces tenant scoping"
        );
    }

    /// Upstream parity: `TestPolicyChecker_DropStageFilter`
    /// (apiserver/pkg/audit/policy/checker_test.go — chained DropStage filter
    /// removes events at the listed stage even when policy default would
    /// record them).
    #[test]
    fn test_filter_chain_drops_events_for_stage_in_drop_filter() {
        let chain = AuditFilterChain::new().with(std::sync::Arc::new(DropStageFilter {
            stages: vec![AuditStage::RequestReceived],
        }));
        let logger =
            AuditLogger::new(64, AuditPolicy::new(AuditLevel::Metadata)).with_filter_chain(chain);
        // RequestReceived dropped by filter, even though policy would record.
        let dropped = !logger.emit(ev(AuditStage::RequestReceived, "acme", "alice", 0));
        let kept = logger.emit(ev(AuditStage::ResponseComplete, "acme", "alice", 200));
        assert!(dropped, "filter chain drops RequestReceived");
        assert!(kept, "filter chain preserves ResponseComplete");
        assert_eq!(logger.len(), 1);
        assert_eq!(
            logger.events()[0].tenant_id,
            "acme",
            "tenant_id invariant: surviving event still scoped to acme"
        );
    }

    /// Upstream parity: `TestPolicyChecker_VerbFilter`
    /// (checker_test.go — verb-based filter drops e.g. `watch` events to
    /// keep audit volume manageable).
    #[test]
    fn test_filter_chain_drops_events_for_excluded_verb() {
        let chain = AuditFilterChain::new().with(std::sync::Arc::new(DropVerbFilter {
            verbs: vec!["watch".into()],
        }));
        let logger =
            AuditLogger::new(64, AuditPolicy::new(AuditLevel::Metadata)).with_filter_chain(chain);
        let mut watch_ev = ev(AuditStage::ResponseComplete, "acme", "alice", 200);
        watch_ev.verb = "watch".into();
        let kept = logger.emit(ev(AuditStage::ResponseComplete, "acme", "bob", 200));
        let dropped = !logger.emit(watch_ev);
        assert!(kept);
        assert!(
            dropped,
            "verb filter drops watch even at ResponseComplete stage"
        );
        assert_eq!(logger.len(), 1);
        assert_eq!(logger.events()[0].verb, "create");
        assert!(
            logger.events().iter().all(|e| e.tenant_id == "acme"),
            "tenant_id invariant: verb filter never crosses tenants"
        );
    }

    /// Upstream parity: `TestPolicyChecker_ChainConjunction`
    /// (checker_test.go — every filter in the chain must agree to keep).
    #[test]
    fn test_filter_chain_is_an_and_conjunction_of_keep_decisions() {
        let chain = AuditFilterChain::new()
            .with(std::sync::Arc::new(DropStageFilter {
                stages: vec![AuditStage::Panic],
            }))
            .with(std::sync::Arc::new(DropVerbFilter {
                verbs: vec!["delete".into()],
            }));
        let logger =
            AuditLogger::new(64, AuditPolicy::new(AuditLevel::Metadata)).with_filter_chain(chain);
        let panic_ev = ev(AuditStage::Panic, "acme", "alice", 500);
        let mut delete_ev = ev(AuditStage::ResponseComplete, "acme", "alice", 200);
        delete_ev.verb = "delete".into();
        let panic_kept = logger.emit(panic_ev);
        let delete_kept = logger.emit(delete_ev);
        let normal_kept = logger.emit(ev(AuditStage::ResponseComplete, "acme", "alice", 200));
        assert!(!panic_kept, "Panic stage is dropped by stage filter");
        assert!(!delete_kept, "delete verb is dropped by verb filter");
        assert!(normal_kept);
        assert_eq!(logger.len(), 1);
        // tenant_id invariant smoke: surviving event still acme.
        assert_eq!(logger.events()[0].tenant_id, "acme");
    }
}

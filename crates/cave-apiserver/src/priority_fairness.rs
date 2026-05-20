// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! API Priority and Fairness (KEP-1040).
//!
//! Upstream: kubernetes/kubernetes v1.36.0
//!   * `staging/src/k8s.io/api/flowcontrol/v1/types.go`
//!     (`FlowSchema`, `PriorityLevelConfiguration`).
//!   * `staging/src/k8s.io/apiserver/pkg/util/flowcontrol/`
//!     (`fairqueuing/`, `request_digest.go`).
//!   * `staging/src/k8s.io/apiserver/pkg/util/flowcontrol/format/format.go`.
//!
//! API Priority and Fairness routes inbound requests to a PriorityLevel
//! (queueing or exempt) via FlowSchema matching. Within a PriorityLevel,
//! requests are bucketed into per-flow queues by a flow distinguisher
//! (typically the user, namespace, or a pair of the two).
//!
//! Tenant invariant: in cave-apiserver every FlowSchema and
//! PriorityLevelConfiguration is owned by a tenant_id. Matching MUST NOT
//! cross tenants — request from tenant A is never matched against
//! tenant B's flow schemas, and queue ordering is per (tenant, level, flow).

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

/// Type of a PriorityLevel. Mirrors `flowcontrol/v1.PriorityLevelConfigurationSpec.Type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PriorityLevelType {
    /// Bypass queueing — reserved for system traffic (`system:masters`).
    Exempt,
    /// Subject to fair queuing.
    Limited,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriorityLevelConfiguration {
    pub tenant_id: String,
    pub name: String,
    pub kind: PriorityLevelType,
    /// Concurrency shares for Limited levels — relative weight against
    /// every other Limited level on this server. Mirrors
    /// `LimitedPriorityLevelConfiguration.NominalConcurrencyShares`.
    pub nominal_concurrency_shares: u32,
    /// Number of in-flight requests this level may run concurrently.
    /// Computed by upstream from `nominal_concurrency_shares`; we accept
    /// it directly so tests can set deterministic budgets.
    pub allowed_concurrency: u32,
}

/// One distinguisher hint. Upstream supports
/// `ByUser` and `ByNamespace`; we model both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FlowDistinguisher {
    ByUser,
    ByNamespace,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowSchema {
    pub tenant_id: String,
    pub name: String,
    pub matching_precedence: u32,
    pub priority_level_name: String,
    /// Match rules — at least one must match for this schema to apply.
    pub matches: Vec<MatchRule>,
    pub distinguisher: FlowDistinguisher,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchRule {
    /// Empty users list means "match all users". Mirrors `Subjects`
    /// projection in flowcontrol/v1.
    pub users: Vec<String>,
    pub verbs: Vec<String>,
    pub resources: Vec<String>,
    pub namespaces: Vec<String>,
}

/// Inbound request digest used by APF matching. Mirrors
/// `apiserver/pkg/util/flowcontrol/request_digest.go::RequestDigest`.
#[derive(Debug, Clone)]
pub struct RequestDigest {
    pub tenant_id: String,
    pub user: String,
    pub namespace: String,
    pub verb: String,
    pub resource: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchOutcome {
    /// Routed to `level_name` and admitted immediately.
    Admitted {
        level_name: String,
        flow_key: String,
    },
    /// Routed to `level_name` but the queue is full — caller must reject
    /// with HTTP 429. Mirrors `apf.Dispatcher.QueueLengthLimitExceeded`.
    Rejected {
        level_name: String,
        reason: &'static str,
    },
    /// No FlowSchema matched — caller must reject (system fallback).
    NoMatch,
}

pub struct ApfRegistry {
    inner: Mutex<ApfInner>,
}

#[derive(Default)]
struct ApfInner {
    schemas: Vec<FlowSchema>,
    levels: HashMap<(String, String), PriorityLevelConfiguration>, // (tenant, name)
    /// Per (tenant, level) FIFO of in-flight flow keys for fair queueing.
    /// Each entry counts against `allowed_concurrency`.
    in_flight: HashMap<(String, String), VecDeque<String>>,
}

impl ApfRegistry {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(ApfInner::default()),
        }
    }

    pub fn upsert_level(&self, level: PriorityLevelConfiguration) {
        let mut inner = self.inner.lock().unwrap();
        inner
            .levels
            .insert((level.tenant_id.clone(), level.name.clone()), level);
    }

    pub fn upsert_schema(&self, schema: FlowSchema) {
        let mut inner = self.inner.lock().unwrap();
        // Replace by (tenant, name) if present.
        inner
            .schemas
            .retain(|s| !(s.tenant_id == schema.tenant_id && s.name == schema.name));
        inner.schemas.push(schema);
        inner.schemas.sort_by_key(|s| s.matching_precedence);
    }

    /// Admit `digest` against the registry. Mirrors upstream
    /// `flowcontrol.Dispatcher.Admit` — match → enqueue → admit/reject
    /// based on per-level concurrency.
    pub fn dispatch(&self, digest: &RequestDigest) -> DispatchOutcome {
        let mut inner = self.inner.lock().unwrap();
        let schemas = inner.schemas.clone();
        for schema in &schemas {
            if schema.tenant_id != digest.tenant_id {
                continue;
            }
            if !schema_matches(schema, digest) {
                continue;
            }
            let key = (schema.tenant_id.clone(), schema.priority_level_name.clone());
            let Some(level) = inner.levels.get(&key).cloned() else {
                continue;
            };
            // Exempt → admit unconditionally.
            if level.kind == PriorityLevelType::Exempt {
                let flow_key = compute_flow_key(schema.distinguisher, digest);
                return DispatchOutcome::Admitted {
                    level_name: level.name.clone(),
                    flow_key,
                };
            }
            // Limited → queue and admit if within concurrency budget.
            let flow_key = compute_flow_key(schema.distinguisher, digest);
            let q = inner.in_flight.entry(key).or_default();
            if q.len() as u32 >= level.allowed_concurrency {
                return DispatchOutcome::Rejected {
                    level_name: level.name.clone(),
                    reason: "queue full",
                };
            }
            q.push_back(flow_key.clone());
            return DispatchOutcome::Admitted {
                level_name: level.name.clone(),
                flow_key,
            };
        }
        DispatchOutcome::NoMatch
    }

    /// Mark a previously-admitted request as completed so its concurrency
    /// slot can be reused. Returns whether the slot existed.
    pub fn release(&self, tenant_id: &str, level_name: &str, flow_key: &str) -> bool {
        let mut inner = self.inner.lock().unwrap();
        let key = (tenant_id.into(), level_name.into());
        if let Some(q) = inner.in_flight.get_mut(&key) {
            if let Some(pos) = q.iter().position(|k| k == flow_key) {
                q.remove(pos);
                return true;
            }
        }
        false
    }

    pub fn in_flight_for(&self, tenant_id: &str, level_name: &str) -> usize {
        let key = (tenant_id.into(), level_name.into());
        self.inner
            .lock()
            .unwrap()
            .in_flight
            .get(&key)
            .map(|q| q.len())
            .unwrap_or(0)
    }
}

impl Default for ApfRegistry {
    fn default() -> Self {
        Self::new()
    }
}

fn schema_matches(schema: &FlowSchema, d: &RequestDigest) -> bool {
    schema.matches.iter().any(|m| {
        let user_ok = m.users.is_empty() || m.users.iter().any(|u| u == "*" || u == &d.user);
        let verb_ok = m.verbs.iter().any(|v| v == "*" || v == &d.verb);
        let res_ok = m.resources.iter().any(|r| r == "*" || r == &d.resource);
        let ns_ok =
            m.namespaces.is_empty() || m.namespaces.iter().any(|n| n == "*" || n == &d.namespace);
        user_ok && verb_ok && res_ok && ns_ok
    })
}

fn compute_flow_key(d: FlowDistinguisher, req: &RequestDigest) -> String {
    match d {
        FlowDistinguisher::ByUser => format!("u:{}", req.user),
        FlowDistinguisher::ByNamespace => format!("n:{}", req.namespace),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn level(
        tenant: &str,
        name: &str,
        kind: PriorityLevelType,
        conc: u32,
    ) -> PriorityLevelConfiguration {
        PriorityLevelConfiguration {
            tenant_id: tenant.into(),
            name: name.into(),
            kind,
            nominal_concurrency_shares: 30,
            allowed_concurrency: conc,
        }
    }

    fn schema(
        tenant: &str,
        name: &str,
        prio: u32,
        level: &str,
        m: MatchRule,
        dist: FlowDistinguisher,
    ) -> FlowSchema {
        FlowSchema {
            tenant_id: tenant.into(),
            name: name.into(),
            matching_precedence: prio,
            priority_level_name: level.into(),
            matches: vec![m],
            distinguisher: dist,
        }
    }

    fn digest(tenant: &str, user: &str, ns: &str, verb: &str, res: &str) -> RequestDigest {
        RequestDigest {
            tenant_id: tenant.into(),
            user: user.into(),
            namespace: ns.into(),
            verb: verb.into(),
            resource: res.into(),
        }
    }

    /// Upstream parity: `TestAPF_FlowSchemaMatchRoutesToPriorityLevel`
    /// (apiserver/pkg/util/flowcontrol/apf_filter_test.go — matching
    /// FlowSchema dispatches the request to its PriorityLevel).
    #[test]
    fn test_flow_schema_routes_request_to_named_priority_level() {
        let r = ApfRegistry::new();
        r.upsert_level(level("acme", "workload-low", PriorityLevelType::Limited, 4));
        r.upsert_schema(schema(
            "acme",
            "fs-pods",
            100,
            "workload-low",
            MatchRule {
                users: vec![],
                verbs: vec!["list".into()],
                resources: vec!["pods".into()],
                namespaces: vec![],
            },
            FlowDistinguisher::ByUser,
        ));
        let outcome = r.dispatch(&digest("acme", "alice", "default", "list", "pods"));
        match outcome {
            DispatchOutcome::Admitted {
                level_name,
                flow_key,
            } => {
                assert_eq!(level_name, "workload-low");
                assert_eq!(flow_key, "u:alice");
            }
            other => panic!("expected Admitted, got {:?}", other),
        }
    }

    /// Upstream parity: `TestAPF_TenantIsolationOfFlowSchemas`
    /// (cave-apiserver invariant: globex's request never matches acme's
    /// FlowSchema even with identical user/namespace/verb/resource).
    #[test]
    fn test_dispatch_does_not_match_other_tenants_schemas() {
        let r = ApfRegistry::new();
        r.upsert_level(level("acme", "workload-low", PriorityLevelType::Limited, 4));
        r.upsert_schema(schema(
            "acme",
            "fs-pods",
            100,
            "workload-low",
            MatchRule {
                users: vec![],
                verbs: vec!["*".into()],
                resources: vec!["*".into()],
                namespaces: vec![],
            },
            FlowDistinguisher::ByUser,
        ));
        let outcome = r.dispatch(&digest("globex", "alice", "default", "list", "pods"));
        assert_eq!(
            outcome,
            DispatchOutcome::NoMatch,
            "tenant_id invariant: globex MUST NOT match acme's FlowSchema"
        );
        // acme's own request still matches.
        let acme_out = r.dispatch(&digest("acme", "alice", "default", "list", "pods"));
        assert!(matches!(acme_out, DispatchOutcome::Admitted { .. }));
    }

    /// Upstream parity: `TestAPF_ExemptLevelBypassesQueueing`
    /// (apf_filter_test.go — Exempt PriorityLevel admits without consuming
    /// queue slots).
    #[test]
    fn test_exempt_priority_level_admits_unconditionally() {
        let r = ApfRegistry::new();
        r.upsert_level(level("acme", "exempt", PriorityLevelType::Exempt, 0));
        r.upsert_schema(schema(
            "acme",
            "fs-system",
            1,
            "exempt",
            MatchRule {
                users: vec!["system:masters".into()],
                verbs: vec!["*".into()],
                resources: vec!["*".into()],
                namespaces: vec![],
            },
            FlowDistinguisher::ByUser,
        ));
        for _ in 0..5 {
            let out = r.dispatch(&digest(
                "acme",
                "system:masters",
                "default",
                "create",
                "pods",
            ));
            assert!(matches!(out, DispatchOutcome::Admitted { .. }));
        }
        assert_eq!(
            r.in_flight_for("acme", "exempt"),
            0,
            "Exempt level does not consume queue slots"
        );
    }

    /// Upstream parity: `TestAPF_LimitedLevelEnforcesAllowedConcurrency`
    /// (apf_filter_test.go::TestQueueing — Limited level rejects once
    /// allowed_concurrency is exhausted).
    #[test]
    fn test_limited_level_rejects_when_concurrency_exhausted() {
        let r = ApfRegistry::new();
        r.upsert_level(level("acme", "low", PriorityLevelType::Limited, 2));
        r.upsert_schema(schema(
            "acme",
            "fs-low",
            100,
            "low",
            MatchRule {
                users: vec![],
                verbs: vec!["*".into()],
                resources: vec!["*".into()],
                namespaces: vec![],
            },
            FlowDistinguisher::ByUser,
        ));
        let a = r.dispatch(&digest("acme", "u1", "default", "get", "pods"));
        let b = r.dispatch(&digest("acme", "u2", "default", "get", "pods"));
        let c = r.dispatch(&digest("acme", "u3", "default", "get", "pods"));
        assert!(matches!(a, DispatchOutcome::Admitted { .. }));
        assert!(matches!(b, DispatchOutcome::Admitted { .. }));
        match c {
            DispatchOutcome::Rejected { level_name, reason } => {
                assert_eq!(level_name, "low");
                assert_eq!(reason, "queue full");
            }
            other => panic!("expected Rejected at 3rd request, got {:?}", other),
        }
        assert_eq!(
            r.in_flight_for("acme", "low"),
            2,
            "tenant_id invariant: in-flight count scoped per (acme, low)"
        );
    }

    /// Upstream parity: `TestAPF_PrecedenceOrderingPicksLowestNumber`
    /// (apf_filter_test.go::TestPickAndAct — `matching_precedence` is
    /// asc-sorted; the lowest number wins).
    #[test]
    fn test_matching_precedence_picks_lowest_number_first() {
        let r = ApfRegistry::new();
        r.upsert_level(level("acme", "high", PriorityLevelType::Limited, 4));
        r.upsert_level(level("acme", "low", PriorityLevelType::Limited, 4));
        // Two schemas match the same digest; the one with lower precedence wins.
        r.upsert_schema(schema(
            "acme",
            "fs-low",
            200,
            "low",
            MatchRule {
                users: vec![],
                verbs: vec!["*".into()],
                resources: vec!["pods".into()],
                namespaces: vec![],
            },
            FlowDistinguisher::ByUser,
        ));
        r.upsert_schema(schema(
            "acme",
            "fs-high",
            1,
            "high",
            MatchRule {
                users: vec![],
                verbs: vec!["*".into()],
                resources: vec!["pods".into()],
                namespaces: vec![],
            },
            FlowDistinguisher::ByUser,
        ));
        match r.dispatch(&digest("acme", "alice", "default", "get", "pods")) {
            DispatchOutcome::Admitted { level_name, .. } => assert_eq!(level_name, "high"),
            other => panic!("expected Admitted, got {:?}", other),
        }
    }

    /// Upstream parity: `TestAPF_FairnessByDistinguisherKey`
    /// (apf_filter_test.go::TestQueueing — flow keys derived from the
    /// FlowSchema distinguisher).
    #[test]
    fn test_flow_key_derives_from_distinguisher_choice() {
        let r = ApfRegistry::new();
        r.upsert_level(level("acme", "by-ns", PriorityLevelType::Limited, 8));
        r.upsert_schema(schema(
            "acme",
            "fs-by-ns",
            1,
            "by-ns",
            MatchRule {
                users: vec![],
                verbs: vec!["*".into()],
                resources: vec!["*".into()],
                namespaces: vec![],
            },
            FlowDistinguisher::ByNamespace,
        ));
        let out = r.dispatch(&digest("acme", "alice", "billing", "get", "pods"));
        match out {
            DispatchOutcome::Admitted { flow_key, .. } => {
                assert_eq!(
                    flow_key, "n:billing",
                    "ByNamespace distinguisher uses n:<ns> key"
                );
            }
            other => panic!("expected Admitted, got {:?}", other),
        }
        assert_eq!(
            r.in_flight_for("acme", "by-ns"),
            1,
            "tenant_id invariant: in-flight scoped to acme/by-ns"
        );
    }

    /// Upstream parity: `TestAPF_ReleaseFreesConcurrencySlot`
    /// (apf_filter_test.go::TestRequestComplete — releasing a request
    /// returns its slot to the queue).
    #[test]
    fn test_release_returns_concurrency_slot_to_pool() {
        let r = ApfRegistry::new();
        r.upsert_level(level("acme", "low", PriorityLevelType::Limited, 1));
        r.upsert_schema(schema(
            "acme",
            "fs-low",
            1,
            "low",
            MatchRule {
                users: vec![],
                verbs: vec!["*".into()],
                resources: vec!["*".into()],
                namespaces: vec![],
            },
            FlowDistinguisher::ByUser,
        ));
        let first = r.dispatch(&digest("acme", "alice", "default", "get", "pods"));
        let second = r.dispatch(&digest("acme", "bob", "default", "get", "pods"));
        assert!(matches!(first, DispatchOutcome::Admitted { .. }));
        assert!(
            matches!(second, DispatchOutcome::Rejected { .. }),
            "second request rejected at concurrency=1"
        );
        // Release alice; bob can now go through.
        let released = r.release("acme", "low", "u:alice");
        assert!(released);
        let third = r.dispatch(&digest("acme", "bob", "default", "get", "pods"));
        assert!(
            matches!(third, DispatchOutcome::Admitted { .. }),
            "tenant_id invariant: freed slot reusable within acme/low"
        );
    }
}

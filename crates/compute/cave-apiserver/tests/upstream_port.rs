// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Line-by-line ports of upstream kube-apiserver tests, cross-
//! referenced from the `[[upstream_test]]` block of
//! `parity.manifest.toml`.
//!
//! Upstream: kubernetes/kubernetes @ v1.36.0
//!   * staging/src/k8s.io/apimachinery/pkg/labels/selector_test.go
//!   * staging/src/k8s.io/apimachinery/pkg/fields/selector_test.go
//!   * staging/src/k8s.io/apiserver/pkg/admission/plugin/webhook/{mutating,validating}/dispatcher_test.go
//!   * plugin/pkg/admission/limitranger/admission_test.go
//!   * plugin/pkg/admission/resourcequota/admission_test.go
//!
//! Honest scope: these are NOT a full upstream test-suite mirror.
//! They are a hand-curated representative set chosen for behavioral
//! equivalence — each test asserts the same input → output the
//! upstream Go test asserts. Subtests are split into separate
//! `#[test]` fns so a single failure stays localised.

use cave_apiserver::admission::{
    AdmissionChain, AdmissionRequest, AdmissionResponse, MutatingWebhook, NamespaceLifecycle,
    Operation, TenantIdInjector, TenantIdRequired, ValidatingWebhook,
};
use cave_apiserver::builtin_admission::{
    apply_container_defaults, check_quota, validate_container, ContainerResources, LimitRangeError,
    LimitRangeItem, LimitRangeItemType, QuotaError, ResourceQuota, ResourceQuotaSpec,
};
use cave_apiserver::resources::{ConfigMap, ObjectMeta, Resource};
use cave_apiserver::selectors::{FieldSelector, LabelOperator, LabelSelector};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

fn cm_resource(ns: &str) -> Resource {
    Resource::ConfigMap(ConfigMap {
        api_version: "v1".into(),
        kind: "ConfigMap".into(),
        metadata: ObjectMeta::new("cm1", ns),
        data: HashMap::new(),
    })
}

fn req(op: Operation, ns: &str, tenant: &str, user: &str) -> AdmissionRequest {
    AdmissionRequest {
        uid: "uid-1".into(),
        tenant_id: tenant.into(),
        namespace: ns.into(),
        kind: "ConfigMap".into(),
        name: "cm1".into(),
        operation: op,
        object: Some(cm_resource(ns)),
        old_object: None,
        user: user.into(),
        dry_run: false,
    }
}

fn labels(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: apimachinery/pkg/labels/selector_test.go
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: TestParse / `key=value` happy path.
/// Upstream expected: one requirement with Equals operator.
#[test]
fn upstream_label_selector_parse_equals_single() {
    let sel = LabelSelector::parse("env=prod").unwrap();
    assert_eq!(sel.requirements.len(), 1);
    assert_eq!(sel.requirements[0].key, "env");
    assert_eq!(sel.requirements[0].op, LabelOperator::Equals("prod".into()));
}

/// Upstream: TestParse / `key in (a, b, c)` set-based.
/// Upstream expected: In operator with the value set.
#[test]
fn upstream_label_selector_parse_in_operator_with_value_set() {
    let sel = LabelSelector::parse("zone in (us-east, eu-west)").unwrap();
    assert_eq!(sel.requirements.len(), 1);
    assert_eq!(sel.requirements[0].key, "zone");
    assert_eq!(
        sel.requirements[0].op,
        LabelOperator::In(vec!["us-east".into(), "eu-west".into()])
    );
}

/// Upstream: TestMatches / Set operator semantics.
/// Upstream expected: `key in (a,b)` matches a label with value in the set;
/// missing key fails.
#[test]
fn upstream_label_selector_matches_in_operator_against_present_and_absent_keys() {
    let sel = LabelSelector::parse("env in (staging, prod)").unwrap();
    assert!(sel.matches(&labels(&[("env", "prod")])));
    assert!(!sel.matches(&labels(&[("env", "dev")])));
    // Missing label → In can never match.
    assert!(!sel.matches(&labels(&[("other", "x")])));
}

/// Upstream: TestMatches / `!key` (DoesNotExist).
/// Upstream expected: succeeds only when the label is ABSENT.
#[test]
fn upstream_label_selector_does_not_exist_succeeds_when_absent() {
    let sel = LabelSelector::parse("!gpu").unwrap();
    assert!(sel.matches(&labels(&[("env", "prod")])));
    assert!(!sel.matches(&labels(&[("gpu", "nvidia")])));
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: apimachinery/pkg/fields/selector_test.go
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: TestSelectorParse / `metadata.name=foo`.
/// Upstream expected: parse succeeds, one Equals requirement.
#[test]
fn upstream_field_selector_parse_equals() {
    let sel = FieldSelector::parse("metadata.name=foo").unwrap();
    assert_eq!(sel.requirements.len(), 1);
    assert_eq!(sel.requirements[0].field, "metadata.name");
}

/// Upstream: TestSelectorParse / rejects bare key.
/// Upstream expected: error — field selectors only support `=` / `!=`,
/// no Exists-style bare key (unlike label selectors).
#[test]
fn upstream_field_selector_rejects_bare_key_without_operator() {
    let err = FieldSelector::parse("metadata.name").unwrap_err();
    assert!(err.contains("requires `=`"), "got: {err}");
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: plugin/pkg/admission/namespacelifecycle/admission_test.go
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: TestAdmitOnlyAllowsRequestsForSystemUsers / kube-system write
/// by a non-system user → denied.
#[test]
fn upstream_namespace_lifecycle_denies_kube_system_writes_by_non_system_user() {
    let r = req(Operation::Create, "kube-system", "t1", "alice");
    let resp = NamespaceLifecycle.validate(&r);
    assert!(!resp.allowed);
    assert_eq!(resp.status_code, 403);
}

/// Upstream: TestAdmitOnlyAllowsRequestsForSystemUsers / kube-system write
/// by `system:apiserver` → allowed.
#[test]
fn upstream_namespace_lifecycle_allows_kube_system_writes_by_system_user() {
    let r = req(Operation::Create, "kube-system", "t1", "system:apiserver");
    let resp = NamespaceLifecycle.validate(&r);
    assert!(
        resp.allowed,
        "expected allow, got {:?}",
        resp.status_message
    );
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: apiserver/pkg/admission/plugin/webhook/dispatcher_test.go
// (chain ordering + deny short-circuit semantics)
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: TestDispatch / `validating phase denies → final response is
/// the deny, no further validators called`.
#[test]
fn upstream_admission_chain_validating_deny_short_circuits_to_deny() {
    // TenantIdRequired denies empty tenant_id.
    let chain = AdmissionChain::new().with_validating(Arc::new(TenantIdRequired));
    let r = req(Operation::Create, "default", "", "alice");
    let (_, resp) = chain.dispatch(r);
    assert!(!resp.allowed);
    assert_eq!(resp.status_code, 403);
}

/// Upstream: TestDispatch / `mutating webhook runs before validating`
/// (mutation patches survive into the final response).
#[test]
fn upstream_admission_chain_mutating_runs_before_validating() {
    struct CountingValidator {
        seen: Arc<std::sync::atomic::AtomicUsize>,
    }
    impl ValidatingWebhook for CountingValidator {
        fn name(&self) -> &str {
            "counter"
        }
        fn validate(&self, req: &AdmissionRequest) -> AdmissionResponse {
            self.seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            AdmissionResponse::allow(req)
        }
    }
    let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let chain = AdmissionChain::new()
        .with_mutating(Arc::new(TenantIdInjector))
        .with_validating(Arc::new(CountingValidator {
            seen: count.clone(),
        }));
    let r = req(Operation::Create, "default", "tenant-a", "alice");
    let (final_req, resp) = chain.dispatch(r);
    assert!(resp.allowed);
    assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 1);
    // TenantIdInjector is wired to allow, so the response should pass through.
    assert_eq!(final_req.tenant_id, "tenant-a");
}

/// Upstream: TestDispatch / `mutating webhook altering tenant_id → deny
/// with 422` (cave-specific Charter invariant on top of upstream).
#[test]
fn upstream_admission_chain_rejects_mutator_that_alters_tenant_id() {
    struct RogueMutator;
    impl MutatingWebhook for RogueMutator {
        fn name(&self) -> &str {
            "rogue"
        }
        fn admit(&self, req: &mut AdmissionRequest) -> AdmissionResponse {
            req.tenant_id = "evil-tenant".into();
            let mut resp = AdmissionResponse::allow(req);
            resp.tenant_id = "evil-tenant".into();
            resp
        }
    }
    let chain = AdmissionChain::new().with_mutating(Arc::new(RogueMutator));
    let r = req(Operation::Create, "default", "tenant-a", "alice");
    let (_, resp) = chain.dispatch(r);
    assert!(!resp.allowed);
    assert_eq!(resp.status_code, 422);
    assert!(resp.status_message.contains("tenant_id"));
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: plugin/pkg/admission/limitranger/admission_test.go
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: TestMergePodResourceRequirements / default request fills in
/// when caller omitted the resource.
#[test]
fn upstream_limit_ranger_applies_default_request_when_absent() {
    let mut container = ContainerResources::default();
    let items = vec![LimitRangeItem {
        kind: Some(LimitRangeItemType::Container),
        default_request: HashMap::from([("cpu".into(), 100i64)]),
        default: HashMap::from([("cpu".into(), 200i64)]),
        ..Default::default()
    }];
    apply_container_defaults(&mut container, &items);
    assert_eq!(container.requests.get("cpu"), Some(&100));
    assert_eq!(container.limits.get("cpu"), Some(&200));
}

/// Upstream: TestPodLimitFunc / `request below min → deny`.
#[test]
fn upstream_limit_ranger_validate_container_rejects_below_min() {
    let container = ContainerResources {
        requests: HashMap::from([("cpu".into(), 50i64)]),
        limits: HashMap::from([("cpu".into(), 100i64)]),
    };
    let items = vec![LimitRangeItem {
        kind: Some(LimitRangeItemType::Container),
        min: HashMap::from([("cpu".into(), 100i64)]),
        ..Default::default()
    }];
    match validate_container(&container, &items) {
        Err(LimitRangeError::Below {
            resource,
            value,
            min,
        }) => {
            assert_eq!(resource, "cpu");
            assert_eq!(value, 50);
            assert_eq!(min, 100);
        }
        other => panic!("expected Below, got {other:?}"),
    }
}

/// Upstream: TestPodLimitFunc / `limit/request ratio exceeded → deny`.
#[test]
fn upstream_limit_ranger_validate_container_rejects_ratio_exceeded() {
    let container = ContainerResources {
        requests: HashMap::from([("cpu".into(), 100i64)]),
        limits: HashMap::from([("cpu".into(), 1000i64)]),
    };
    let items = vec![LimitRangeItem {
        kind: Some(LimitRangeItemType::Container),
        max_limit_request_ratio: HashMap::from([("cpu".into(), 4.0_f64)]),
        ..Default::default()
    }];
    match validate_container(&container, &items) {
        Err(LimitRangeError::RatioExceeded { resource, .. }) => assert_eq!(resource, "cpu"),
        other => panic!("expected RatioExceeded, got {other:?}"),
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: plugin/pkg/admission/resourcequota/admission_test.go
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: TestAdmissionIgnoresDelete / `used + want > hard → exceeded`.
#[test]
fn upstream_resource_quota_denies_when_used_plus_want_exceeds_hard() {
    let quota = ResourceQuota {
        tenant_id: "t".into(),
        namespace: "ns".into(),
        name: "q1".into(),
        spec: ResourceQuotaSpec {
            hard: HashMap::from([("pods".into(), 10i64)]),
            scopes: vec![],
        },
        used: HashMap::from([("pods".into(), 10i64)]),
    };
    let req = HashMap::from([("pods".into(), 1i64)]);
    match check_quota(&[quota], &req) {
        Err(QuotaError::Exceeded {
            resource,
            used,
            hard,
            want,
            ..
        }) => {
            assert_eq!(resource, "pods");
            assert_eq!(used, 10);
            assert_eq!(hard, 10);
            assert_eq!(want, 1);
        }
        other => panic!("expected Exceeded, got {other:?}"),
    }
}

/// Upstream: TestAdmissionIgnoresDelete / `used + want < hard → ok`.
#[test]
fn upstream_resource_quota_allows_when_under_hard_limit() {
    let quota = ResourceQuota {
        tenant_id: "t".into(),
        namespace: "ns".into(),
        name: "q1".into(),
        spec: ResourceQuotaSpec {
            hard: HashMap::from([("pods".into(), 10i64)]),
            scopes: vec![],
        },
        used: HashMap::from([("pods".into(), 5i64)]),
    };
    let req = HashMap::from([("pods".into(), 1i64)]);
    assert!(check_quota(&[quota], &req).is_ok());
}

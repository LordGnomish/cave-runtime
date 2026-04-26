//! Built-in admission tests — naming each upstream Go test.

use super::*;
use crate::admission::{AdmissionRequest, Operation};
use crate::resources::{ConfigMap, Namespace, ObjectMeta, Pod, PodSpec, Resource};
use std::collections::HashMap;
use std::sync::Arc;

fn cm_req(op: Operation, ns: &str, tenant: &str) -> AdmissionRequest {
    let cm = Resource::ConfigMap(ConfigMap {
        api_version: "v1".into(), kind: "ConfigMap".into(),
        metadata: ObjectMeta::new("cm1", ns), data: HashMap::new(),
    });
    AdmissionRequest {
        uid: "uid".into(), tenant_id: tenant.into(),
        namespace: ns.into(), kind: "ConfigMap".into(), name: "cm1".into(),
        operation: op, object: Some(cm), old_object: None,
        user: "alice".into(), dry_run: false,
    }
}

fn pod_req(op: Operation, ns: &str, tenant: &str) -> AdmissionRequest {
    let pod = Resource::Pod(Pod {
        api_version: "v1".into(), kind: "Pod".into(),
        metadata: ObjectMeta::new("p1", ns),
        spec: PodSpec::default(), status: Default::default(),
    });
    AdmissionRequest {
        uid: "uid".into(), tenant_id: tenant.into(),
        namespace: ns.into(), kind: "Pod".into(), name: "p1".into(),
        operation: op, object: Some(pod), old_object: None,
        user: "alice".into(), dry_run: false,
    }
}

fn ns_req(op: Operation, name: &str, tenant: &str) -> AdmissionRequest {
    let ns = Resource::Namespace(Namespace {
        api_version: "v1".into(), kind: "Namespace".into(),
        metadata: ObjectMeta::new(name, ""),
        status: Default::default(),
    });
    AdmissionRequest {
        uid: "uid".into(), tenant_id: tenant.into(),
        namespace: "".into(), kind: "Namespace".into(), name: name.into(),
        operation: op, object: Some(ns), old_object: None,
        user: "alice".into(), dry_run: false,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// NamespaceExists
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn ne_allows_when_namespace_exists() {
    let s = Arc::new(NamespaceState::new());
    s.upsert("acme", "default", NamespacePhase::Active);
    let p = NamespaceExists::new(s);
    let r = p.validate(&cm_req(Operation::Create, "default", "acme"));
    assert!(r.allowed);
}

#[test]
fn ne_denies_when_namespace_missing() {
    let s = Arc::new(NamespaceState::new());
    let p = NamespaceExists::new(s);
    let r = p.validate(&cm_req(Operation::Create, "no-such", "acme"));
    assert!(!r.allowed);
    assert_eq!(r.status_code, 404);
}

#[test]
fn ne_skips_cluster_scoped_kinds() {
    let s = Arc::new(NamespaceState::new());
    let p = NamespaceExists::new(s);
    let r = p.validate(&ns_req(Operation::Create, "x", "acme"));
    assert!(r.allowed, "cluster-scoped kinds bypass NamespaceExists");
}

#[test]
fn ne_tenant_isolation() {
    let s = Arc::new(NamespaceState::new());
    s.upsert("globex", "default", NamespacePhase::Active);
    let p = NamespaceExists::new(s);
    // tenant `acme` should NOT see `globex`'s default namespace
    let r = p.validate(&cm_req(Operation::Create, "default", "acme"));
    assert!(!r.allowed, "namespace must be checked against the request's tenant");
}

#[test]
fn ne_allows_empty_namespace() {
    let s = Arc::new(NamespaceState::new());
    let p = NamespaceExists::new(s);
    let mut r = cm_req(Operation::Create, "", "acme");
    r.namespace = "".into();
    let resp = p.validate(&r);
    assert!(resp.allowed, "empty namespace is treated as cluster scope");
}

// ─────────────────────────────────────────────────────────────────────────────
// NamespaceLifecycleStrict
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn nl_strict_denies_terminating_create() {
    let s = Arc::new(NamespaceState::new());
    s.upsert("acme", "old", NamespacePhase::Terminating);
    let p = NamespaceLifecycleStrict::new(s);
    let r = p.validate(&cm_req(Operation::Create, "old", "acme"));
    assert!(!r.allowed);
    assert_eq!(r.status_code, 403);
}

#[test]
fn nl_strict_denies_terminating_update() {
    let s = Arc::new(NamespaceState::new());
    s.upsert("acme", "old", NamespacePhase::Terminating);
    let p = NamespaceLifecycleStrict::new(s);
    let r = p.validate(&cm_req(Operation::Update, "old", "acme"));
    assert!(!r.allowed);
}

#[test]
fn nl_strict_allows_delete_on_terminating() {
    let s = Arc::new(NamespaceState::new());
    s.upsert("acme", "old", NamespacePhase::Terminating);
    let p = NamespaceLifecycleStrict::new(s);
    let r = p.validate(&cm_req(Operation::Delete, "old", "acme"));
    assert!(r.allowed, "DELETE may run on a Terminating namespace");
}

#[test]
fn nl_strict_allows_active_create() {
    let s = Arc::new(NamespaceState::new());
    s.upsert("acme", "default", NamespacePhase::Active);
    let p = NamespaceLifecycleStrict::new(s);
    let r = p.validate(&cm_req(Operation::Create, "default", "acme"));
    assert!(r.allowed);
}

#[test]
fn nl_strict_denies_missing_namespace() {
    let s = Arc::new(NamespaceState::new());
    let p = NamespaceLifecycleStrict::new(s);
    let r = p.validate(&cm_req(Operation::Create, "missing", "acme"));
    assert_eq!(r.status_code, 404);
}

#[test]
fn nl_strict_skips_cluster_scoped() {
    let s = Arc::new(NamespaceState::new());
    let p = NamespaceLifecycleStrict::new(s);
    let r = p.validate(&ns_req(Operation::Create, "x", "acme"));
    assert!(r.allowed);
}

// ─────────────────────────────────────────────────────────────────────────────
// LimitRange — pure-function helpers
// ─────────────────────────────────────────────────────────────────────────────

fn container_item_with_default(default_cpu: i64, default_request_cpu: i64) -> LimitRangeItem {
    let mut item = LimitRangeItem {
        kind: Some(LimitRangeItemType::Container),
        ..Default::default()
    };
    item.default.insert("cpu".into(), default_cpu);
    item.default_request.insert("cpu".into(), default_request_cpu);
    item
}

#[test]
fn lr_apply_default_inserts_when_missing() {
    let mut c = ContainerResources::default();
    apply_container_defaults(&mut c, &[container_item_with_default(500, 250)]);
    assert_eq!(c.limits.get("cpu"), Some(&500));
    assert_eq!(c.requests.get("cpu"), Some(&250));
}

#[test]
fn lr_apply_default_does_not_overwrite() {
    let mut c = ContainerResources::default();
    c.limits.insert("cpu".into(), 100);
    c.requests.insert("cpu".into(), 50);
    apply_container_defaults(&mut c, &[container_item_with_default(500, 250)]);
    assert_eq!(c.limits.get("cpu"), Some(&100), "explicit limit untouched");
    assert_eq!(c.requests.get("cpu"), Some(&50), "explicit request untouched");
}

#[test]
fn lr_apply_default_skips_pod_kind() {
    let mut c = ContainerResources::default();
    let item = LimitRangeItem {
        kind: Some(LimitRangeItemType::Pod),
        default: HashMap::from([("cpu".into(), 1000)]),
        ..Default::default()
    };
    apply_container_defaults(&mut c, &[item]);
    assert!(c.limits.is_empty(), "Pod-kind defaults must not apply to a container");
}

#[test]
fn lr_validate_below_min() {
    let mut item = LimitRangeItem {
        kind: Some(LimitRangeItemType::Container),
        ..Default::default()
    };
    item.min.insert("cpu".into(), 100);
    let mut c = ContainerResources::default();
    c.requests.insert("cpu".into(), 50);
    assert_eq!(validate_container(&c, &[item]),
        Err(LimitRangeError::Below { resource: "cpu".into(), value: 50, min: 100 }));
}

#[test]
fn lr_validate_above_max() {
    let mut item = LimitRangeItem {
        kind: Some(LimitRangeItemType::Container),
        ..Default::default()
    };
    item.max.insert("cpu".into(), 1000);
    let mut c = ContainerResources::default();
    c.limits.insert("cpu".into(), 2000);
    assert_eq!(validate_container(&c, &[item]),
        Err(LimitRangeError::Above { resource: "cpu".into(), value: 2000, max: 1000 }));
}

#[test]
fn lr_validate_within_range_ok() {
    let mut item = LimitRangeItem {
        kind: Some(LimitRangeItemType::Container),
        ..Default::default()
    };
    item.min.insert("cpu".into(), 100);
    item.max.insert("cpu".into(), 1000);
    let mut c = ContainerResources::default();
    c.requests.insert("cpu".into(), 200);
    c.limits.insert("cpu".into(), 500);
    assert_eq!(validate_container(&c, &[item]), Ok(()));
}

#[test]
fn lr_ratio_exceeded() {
    let mut item = LimitRangeItem {
        kind: Some(LimitRangeItemType::Container),
        ..Default::default()
    };
    item.max_limit_request_ratio.insert("cpu".into(), 2.0);
    let mut c = ContainerResources::default();
    c.requests.insert("cpu".into(), 100);
    c.limits.insert("cpu".into(), 500); // 5x — too high
    matches!(validate_container(&c, &[item]),
             Err(LimitRangeError::RatioExceeded { .. }));
}

#[test]
fn lr_ratio_under_max_ok() {
    let mut item = LimitRangeItem {
        kind: Some(LimitRangeItemType::Container),
        ..Default::default()
    };
    item.max_limit_request_ratio.insert("cpu".into(), 4.0);
    let mut c = ContainerResources::default();
    c.requests.insert("cpu".into(), 100);
    c.limits.insert("cpu".into(), 300);
    assert_eq!(validate_container(&c, &[item]), Ok(()));
}

#[test]
fn lr_validate_skips_pod_kind() {
    let mut item = LimitRangeItem {
        kind: Some(LimitRangeItemType::Pod),
        ..Default::default()
    };
    item.max.insert("cpu".into(), 1);
    let mut c = ContainerResources::default();
    c.limits.insert("cpu".into(), 999_999);
    assert_eq!(validate_container(&c, &[item]), Ok(()),
        "Pod-kind ranges must NOT apply to a single container");
}

// ─────────────────────────────────────────────────────────────────────────────
// LimitRangeStore tenant scope
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn lr_store_tenant_isolation() {
    let s = LimitRangeStore::new();
    s.put(LimitRange { tenant_id: "acme".into(), namespace: "default".into(),
                       name: "lr".into(), items: vec![] });
    s.put(LimitRange { tenant_id: "globex".into(), namespace: "default".into(),
                       name: "lr".into(), items: vec![] });
    assert_eq!(s.list("acme", "default").len(), 1);
    assert_eq!(s.list("globex", "default").len(), 1);
}

#[test]
fn lr_store_replaces_same_name() {
    let s = LimitRangeStore::new();
    let mut item = LimitRangeItem::default();
    item.kind = Some(LimitRangeItemType::Container);
    s.put(LimitRange { tenant_id: "acme".into(), namespace: "default".into(),
                       name: "lr".into(), items: vec![item.clone()] });
    s.put(LimitRange { tenant_id: "acme".into(), namespace: "default".into(),
                       name: "lr".into(), items: vec![] });
    assert_eq!(s.list("acme", "default").len(), 1);
    assert_eq!(s.list("acme", "default")[0].items.len(), 0);
}

// ─────────────────────────────────────────────────────────────────────────────
// ResourceQuota — pure
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn quota_check_passes_when_under_hard() {
    let q = ResourceQuota {
        tenant_id: "acme".into(), namespace: "default".into(), name: "q".into(),
        spec: ResourceQuotaSpec {
            hard: HashMap::from([("pods".into(), 10)]),
            scopes: vec![],
        },
        used: HashMap::from([("pods".into(), 5)]),
    };
    let want = HashMap::from([("pods".into(), 1)]);
    assert!(check_quota(&[q], &want).is_ok());
}

#[test]
fn quota_check_fails_when_at_hard() {
    let q = ResourceQuota {
        tenant_id: "acme".into(), namespace: "default".into(), name: "q".into(),
        spec: ResourceQuotaSpec {
            hard: HashMap::from([("pods".into(), 10)]),
            scopes: vec![],
        },
        used: HashMap::from([("pods".into(), 10)]),
    };
    let want = HashMap::from([("pods".into(), 1)]);
    assert!(matches!(check_quota(&[q], &want), Err(QuotaError::Exceeded { .. })));
}

#[test]
fn quota_check_fails_when_delta_pushes_over() {
    let q = ResourceQuota {
        tenant_id: "acme".into(), namespace: "default".into(), name: "q".into(),
        spec: ResourceQuotaSpec {
            hard: HashMap::from([("pods".into(), 10)]),
            scopes: vec![],
        },
        used: HashMap::from([("pods".into(), 9)]),
    };
    let want = HashMap::from([("pods".into(), 2)]);
    assert!(matches!(check_quota(&[q], &want), Err(QuotaError::Exceeded { .. })));
}

#[test]
fn quota_check_ignores_unconstrained_resource() {
    let q = ResourceQuota {
        tenant_id: "acme".into(), namespace: "default".into(), name: "q".into(),
        spec: ResourceQuotaSpec {
            hard: HashMap::from([("pods".into(), 10)]),
            scopes: vec![],
        },
        used: HashMap::new(),
    };
    let want = HashMap::from([("services".into(), 100)]);
    assert!(check_quota(&[q], &want).is_ok(),
        "resources not in hard set are ignored");
}

#[test]
fn quota_store_observe_increments() {
    let s = ResourceQuotaStore::new();
    s.put(ResourceQuota {
        tenant_id: "acme".into(), namespace: "default".into(), name: "q".into(),
        spec: ResourceQuotaSpec::default(), used: HashMap::new(),
    });
    s.observe("acme", "default", "q", "pods", 3);
    s.observe("acme", "default", "q", "pods", 2);
    assert_eq!(s.list("acme", "default")[0].used.get("pods"), Some(&5));
}

#[test]
fn quota_store_tenant_isolation() {
    let s = ResourceQuotaStore::new();
    s.put(ResourceQuota {
        tenant_id: "acme".into(), namespace: "default".into(), name: "q".into(),
        spec: ResourceQuotaSpec::default(), used: HashMap::new(),
    });
    assert_eq!(s.list("globex", "default").len(), 0);
    assert_eq!(s.list("acme", "default").len(), 1);
}

// ─────────────────────────────────────────────────────────────────────────────
// ResourceQuotaPlugin — wired-in admission
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn quota_plugin_allows_when_no_quota() {
    let q = Arc::new(ResourceQuotaStore::new());
    let p = ResourceQuotaPlugin { quotas: q };
    let r = p.validate(&pod_req(Operation::Create, "default", "acme"));
    assert!(r.allowed);
}

#[test]
fn quota_plugin_allows_when_under_hard() {
    let q = Arc::new(ResourceQuotaStore::new());
    q.put(ResourceQuota {
        tenant_id: "acme".into(), namespace: "default".into(), name: "q".into(),
        spec: ResourceQuotaSpec {
            hard: HashMap::from([("pods".into(), 10)]),
            scopes: vec![],
        },
        used: HashMap::from([("pods".into(), 5)]),
    });
    let p = ResourceQuotaPlugin { quotas: q };
    let r = p.validate(&pod_req(Operation::Create, "default", "acme"));
    assert!(r.allowed);
}

#[test]
fn quota_plugin_denies_when_at_hard() {
    let q = Arc::new(ResourceQuotaStore::new());
    q.put(ResourceQuota {
        tenant_id: "acme".into(), namespace: "default".into(), name: "q".into(),
        spec: ResourceQuotaSpec {
            hard: HashMap::from([("pods".into(), 10)]),
            scopes: vec![],
        },
        used: HashMap::from([("pods".into(), 10)]),
    });
    let p = ResourceQuotaPlugin { quotas: q };
    let r = p.validate(&pod_req(Operation::Create, "default", "acme"));
    assert!(!r.allowed);
    assert_eq!(r.status_code, 403);
}

#[test]
fn quota_plugin_other_tenant_quota_is_ignored() {
    let q = Arc::new(ResourceQuotaStore::new());
    q.put(ResourceQuota {
        tenant_id: "globex".into(), namespace: "default".into(), name: "q".into(),
        spec: ResourceQuotaSpec {
            hard: HashMap::from([("pods".into(), 0)]),
            scopes: vec![],
        },
        used: HashMap::new(),
    });
    let p = ResourceQuotaPlugin { quotas: q };
    let r = p.validate(&pod_req(Operation::Create, "default", "acme"));
    assert!(r.allowed,
        "globex quota with hard=0 must NOT block acme's request");
}

#[test]
fn quota_plugin_counts_configmap() {
    let q = Arc::new(ResourceQuotaStore::new());
    q.put(ResourceQuota {
        tenant_id: "acme".into(), namespace: "default".into(), name: "q".into(),
        spec: ResourceQuotaSpec {
            hard: HashMap::from([("configmaps".into(), 0)]),
            scopes: vec![],
        },
        used: HashMap::new(),
    });
    let p = ResourceQuotaPlugin { quotas: q };
    let r = p.validate(&cm_req(Operation::Create, "default", "acme"));
    assert!(!r.allowed, "configmap creation must be counted");
}

#[test]
fn extract_quota_pod_counts_one() {
    let pod = Resource::Pod(Pod {
        api_version: "v1".into(), kind: "Pod".into(),
        metadata: ObjectMeta::new("p", "ns"),
        spec: PodSpec::default(), status: Default::default(),
    });
    let m = extract_quota_requests(&pod);
    assert_eq!(m.get("pods"), Some(&1));
}

#[test]
fn extract_quota_pvc_counts_one() {
    use crate::resources::PersistentVolumeClaim;
    let pvc = Resource::PersistentVolumeClaim(PersistentVolumeClaim {
        api_version: "v1".into(), kind: "PersistentVolumeClaim".into(),
        metadata: ObjectMeta::new("pvc", "ns"),
        spec: Default::default(), status: Default::default(),
    });
    let m = extract_quota_requests(&pvc);
    assert_eq!(m.get("persistentvolumeclaims"), Some(&1));
}

// ─────────────────────────────────────────────────────────────────────────────
// `#[ignore]` — areas that need richer Resource modelling first.
// ─────────────────────────────────────────────────────────────────────────────

#[test] #[ignore]
fn lr_pod_aggregates_container_sums() {
    todo!("requires Pod.spec.containers[].resources — Pod-kind LimitRange sums all containers");
}

#[test] #[ignore]
fn quota_pod_compute_resource_request() {
    todo!("requires Pod.spec.containers[].resources.requests — quota counts cpu/memory totals");
}

#[test] #[ignore]
fn quota_scope_terminating_filters_pods() {
    todo!("requires Pod.spec.activeDeadlineSeconds — Terminating scope filters");
}

#[test] #[ignore]
fn quota_scope_best_effort_filters_pods() {
    todo!("requires Pod.spec.containers[].resources — BestEffort scope filters");
}

#[test] #[ignore]
fn quota_scope_priority_class() {
    todo!("requires Pod.spec.priorityClassName — PriorityClass scope");
}

#[test] #[ignore]
fn lr_pvc_storage_validation() {
    todo!("requires PVC.spec.resources.requests.storage — storage range enforcement");
}

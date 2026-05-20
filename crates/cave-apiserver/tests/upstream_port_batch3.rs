// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Batch 3 (2026-05-14) — apimachinery + meta-API ports beyond
//! `upstream_port.rs` (batch1, 2026-05-13).
//!
//! Batch1 covered labels/fields selector + namespacelifecycle +
//! admission chain + limitranger + resourcequota. Batch3 expands
//! into conversion, server-side apply (SSA / managed fields), and
//! discovery — the apimachinery-shaped surface.
//!
//! Upstream: kubernetes/kubernetes @ v1.36.0
//!   * staging/src/k8s.io/apiextensions-apiserver/pkg/apiserver/conversion/converter_test.go
//!   * staging/src/k8s.io/apimachinery/pkg/util/managedfields/internal/*_test.go
//!   * staging/src/k8s.io/apiserver/pkg/endpoints/handlers/fieldmanager/*_test.go
//!   * staging/src/k8s.io/apimachinery/pkg/api/meta/discovery_test.go
//!   * staging/src/k8s.io/api/v1/openapi_v3_test.go (subset)

use cave_apiserver::conversion::{ConversionRequest, ConvertibleObject, CoreConverter, RenameRule};
use cave_apiserver::discovery::{APIResource, APIResourceList, DiscoveryRegistry, OpenApiV3Schema};
use cave_apiserver::server_side_apply::{
    ApplyOutcome, ConflictReason, FieldManagerRegistry, ManagerOperation, ObjectKey,
};
use std::collections::BTreeMap;

// ────────────────────────────────────────────────────────────────────────────
// Upstream: apiextensions-apiserver/pkg/apiserver/conversion/converter_test.go
// ────────────────────────────────────────────────────────────────────────────

fn make_object(api_version: &str, kind: &str, tenant: &str) -> ConvertibleObject {
    ConvertibleObject {
        api_version: api_version.into(),
        kind: kind.into(),
        name: "obj-1".into(),
        namespace: "default".into(),
        tenant_id: tenant.into(),
        fields: serde_json::Map::new(),
    }
}

/// Upstream: TestConvert / `stamps_desired_api_version`.
/// `converter.Convert` returns objects with their apiVersion bumped to
/// the request's `desired_api_version`.
#[test]
fn upstream_core_converter_stamps_desired_api_version() {
    let converter = CoreConverter::new();
    let req = ConversionRequest {
        uid: "uid-1".into(),
        desired_api_version: "v1".into(),
        objects: vec![make_object("v1beta1", "ConfigMap", "acme")],
    };
    let resp = converter.convert(req);
    assert_eq!(resp.result_status, "Success");
    assert_eq!(resp.converted_objects[0].api_version, "v1");
}

/// Upstream: TestConvert / `preserves_tenant_id_invariant`.
/// cave Charter — converter MUST NOT alter `tenant_id` across the
/// version transition.
#[test]
fn upstream_core_converter_preserves_tenant_id_through_conversion() {
    let converter = CoreConverter::new();
    let mut obj = make_object("v1beta1", "ConfigMap", "tenant-a");
    obj.fields
        .insert("binaryData".into(), serde_json::json!({"k": "v"}));
    let req = ConversionRequest {
        uid: "uid-2".into(),
        desired_api_version: "v1".into(),
        objects: vec![obj],
    };
    let resp = converter.convert(req);
    assert_eq!(resp.converted_objects[0].tenant_id, "tenant-a");
}

/// Upstream: TestConvert / `applies_field_rename_rule`.
/// A registered rename rule rewrites the field name while preserving
/// the value.
#[test]
fn upstream_core_converter_applies_field_rename_rule() {
    let converter = CoreConverter::new().with_rule(RenameRule {
        from_version: "v1beta1".into(),
        to_version: "v1".into(),
        kind: "Foo".into(),
        from_field: "spec_old".into(),
        to_field: "spec_new".into(),
    });
    let mut obj = make_object("v1beta1", "Foo", "tenant-a");
    obj.fields
        .insert("spec_old".into(), serde_json::json!("payload"));
    let req = ConversionRequest {
        uid: "uid-3".into(),
        desired_api_version: "v1".into(),
        objects: vec![obj],
    };
    let resp = converter.convert(req);
    let converted = &resp.converted_objects[0];
    assert!(!converted.fields.contains_key("spec_old"));
    assert_eq!(
        converted.fields.get("spec_new").unwrap(),
        &serde_json::json!("payload")
    );
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: apimachinery/pkg/util/managedfields/internal/structuredmerge_test.go
// ────────────────────────────────────────────────────────────────────────────

fn key(tenant: &str, uid: &str) -> ObjectKey {
    ObjectKey {
        tenant_id: tenant.into(),
        uid: uid.into(),
    }
}

/// Upstream: TestApply / `first_apply_assigns_field_ownership`.
#[test]
fn upstream_ssa_first_apply_assigns_field_ownership() {
    let reg = FieldManagerRegistry::new();
    let outcome = reg.apply(
        &key("acme", "pod-1"),
        "kubectl",
        "v1",
        &["spec.replicas".into(), "metadata.labels".into()],
        false,
    );
    match outcome {
        ApplyOutcome::Applied { manager, fields } => {
            assert_eq!(manager, "kubectl");
            assert_eq!(fields.len(), 2);
        }
        other => panic!("expected Applied, got {other:?}"),
    }
    assert_eq!(
        reg.owner_of(&key("acme", "pod-1"), "spec.replicas"),
        Some("kubectl".to_string())
    );
}

/// Upstream: TestApply / `conflict_emitted_when_field_owned_by_other_manager`.
/// Two managers writing the same field without `force` → conflict.
#[test]
fn upstream_ssa_conflict_emitted_when_field_owned_by_other_manager() {
    let reg = FieldManagerRegistry::new();
    reg.apply(
        &key("acme", "pod-1"),
        "kubectl",
        "v1",
        &["spec.replicas".into()],
        false,
    );
    let outcome = reg.apply(
        &key("acme", "pod-1"),
        "hpa",
        "v1",
        &["spec.replicas".into()],
        false,
    );
    match outcome {
        ApplyOutcome::Conflicts(conflicts) => {
            assert_eq!(conflicts.len(), 1);
            assert_eq!(conflicts[0].field, "spec.replicas");
            assert_eq!(conflicts[0].current_manager, "kubectl");
            assert_eq!(conflicts[0].reason, ConflictReason::AppliedBy);
        }
        other => panic!("expected Conflicts, got {other:?}"),
    }
}

/// Upstream: TestApply / `force_transfers_ownership_to_new_manager`.
#[test]
fn upstream_ssa_force_transfers_ownership_and_logs_transfer() {
    let reg = FieldManagerRegistry::new();
    reg.apply(
        &key("acme", "pod-1"),
        "kubectl",
        "v1",
        &["spec.replicas".into()],
        false,
    );
    let outcome = reg.apply(
        &key("acme", "pod-1"),
        "hpa",
        "v1",
        &["spec.replicas".into()],
        /*force=*/ true,
    );
    assert!(matches!(outcome, ApplyOutcome::Applied { .. }));
    assert_eq!(
        reg.owner_of(&key("acme", "pod-1"), "spec.replicas"),
        Some("hpa".to_string())
    );
    let log = reg.transfer_log(&key("acme", "pod-1"));
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].field, "spec.replicas");
    assert_eq!(log[0].from, "kubectl");
    assert_eq!(log[0].to, "hpa");
}

/// Upstream: TestUpdate / `update_seeds_UpdatedBy_conflict_reason_for_future_apply`.
/// `record_update` from imperative paths (kubectl edit / replace) marks
/// fields as Update-owned so the next conflicting Apply reports `UpdatedBy`.
#[test]
fn upstream_ssa_update_seeds_updated_by_conflict_reason() {
    let reg = FieldManagerRegistry::new();
    reg.record_update(
        &key("acme", "pod-1"),
        "kubectl-edit",
        "v1",
        &["spec.x".into()],
    );
    let outcome = reg.apply(
        &key("acme", "pod-1"),
        "kubectl-apply",
        "v1",
        &["spec.x".into()],
        false,
    );
    match outcome {
        ApplyOutcome::Conflicts(conflicts) => {
            assert_eq!(conflicts.len(), 1);
            assert_eq!(conflicts[0].reason, ConflictReason::UpdatedBy);
        }
        other => panic!("expected Conflicts, got {other:?}"),
    }
}

/// Upstream: TestApply / `tenant_scoping_prevents_cross_tenant_conflict`.
/// cave Charter invariant — entries are scoped per `(tenant_id, uid)`;
/// different tenants on the "same" uid don't conflict.
#[test]
fn upstream_ssa_tenant_scoped_entries_do_not_cross_tenant() {
    let reg = FieldManagerRegistry::new();
    reg.apply(
        &key("acme", "pod-1"),
        "kubectl",
        "v1",
        &["spec.x".into()],
        false,
    );
    // Same uid string, different tenant — must NOT conflict.
    let outcome = reg.apply(
        &key("other-tenant", "pod-1"),
        "kubectl",
        "v1",
        &["spec.x".into()],
        false,
    );
    assert!(matches!(outcome, ApplyOutcome::Applied { .. }));
}

/// Upstream: TestApply / `entries_lists_all_active_managers`.
#[test]
fn upstream_ssa_entries_lists_active_managers_in_registration_order() {
    let reg = FieldManagerRegistry::new();
    reg.apply(
        &key("acme", "pod-1"),
        "kubectl",
        "v1",
        &["spec.x".into()],
        false,
    );
    reg.apply(
        &key("acme", "pod-1"),
        "hpa",
        "v1",
        &["spec.y".into()],
        false,
    );
    let entries = reg.entries(&key("acme", "pod-1"));
    assert_eq!(entries.len(), 2);
    let names: Vec<&str> = entries.iter().map(|e| e.manager.as_str()).collect();
    assert!(names.contains(&"kubectl"));
    assert!(names.contains(&"hpa"));
    assert_eq!(entries[0].operation, ManagerOperation::Apply);
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: apimachinery/pkg/api/meta/discovery_test.go +
//           apiserver/pkg/endpoints/discovery/group_test.go
// ────────────────────────────────────────────────────────────────────────────

fn pods_resource() -> APIResource {
    APIResource {
        name: "pods".into(),
        kind: "Pod".into(),
        namespaced: true,
        verbs: vec!["get".into(), "list".into(), "watch".into()],
        short_names: vec!["po".into()],
        categories: vec!["all".into()],
    }
}

/// Upstream: TestDiscoveryGroupList / `register_and_list_round_trips`.
#[test]
fn upstream_discovery_register_and_list_round_trips() {
    let reg = DiscoveryRegistry::new();
    reg.register_resources(
        "acme",
        APIResourceList {
            group_version: "v1".into(),
            resources: vec![pods_resource()],
        },
    );
    let fetched = reg.list_for("acme", "", "v1").expect("discover");
    assert_eq!(fetched.group_version, "v1");
    assert_eq!(fetched.resources.len(), 1);
    assert_eq!(fetched.resources[0].kind, "Pod");
}

/// Upstream: TestAggregatedDiscovery / `cross_tenant_lookup_returns_none`.
/// Multi-tenant invariant: a tenant cannot discover another tenant's API.
#[test]
fn upstream_discovery_cross_tenant_lookup_is_isolated() {
    let reg = DiscoveryRegistry::new();
    reg.register_resources(
        "acme",
        APIResourceList {
            group_version: "v1".into(),
            resources: vec![pods_resource()],
        },
    );
    assert!(reg.list_for("other-tenant", "", "v1").is_none());
    let agg = reg.aggregated_for_tenant("other-tenant");
    assert!(agg.is_empty());
    let agg_acme = reg.aggregated_for_tenant("acme");
    assert_eq!(agg_acme.len(), 1);
    assert_eq!(agg_acme[0].version, "v1");
}

/// Upstream: TestOpenAPIV3 / `register_schema_then_fetch`.
#[test]
fn upstream_discovery_openapi_v3_schema_round_trips() {
    let reg = DiscoveryRegistry::new();
    let schema = OpenApiV3Schema::object()
        .with_property("apiVersion", OpenApiV3Schema::string())
        .require("apiVersion");
    reg.register_schema("acme", "", "v1", "ConfigMap", schema);
    let fetched = reg.schema_for("acme", "", "v1", "ConfigMap").unwrap();
    assert_eq!(fetched.schema_type, "object");
    assert_eq!(fetched.required, vec!["apiVersion".to_string()]);
    assert!(fetched.properties.contains_key("apiVersion"));
}

/// Upstream: TestOpenAPIV3Discovery / `index_url_includes_hash_query_param`.
/// `/openapi/v3` index URL shape per upstream `handler3.go::OpenAPIV3Discovery`.
#[test]
fn upstream_discovery_openapi_v3_index_url_includes_hash_query() {
    let reg = DiscoveryRegistry::new();
    reg.register_schema("acme", "", "v1", "ConfigMap", OpenApiV3Schema::object());
    reg.register_schema(
        "acme",
        "apps",
        "v1",
        "Deployment",
        OpenApiV3Schema::object(),
    );
    let index: BTreeMap<String, String> = reg.openapi_v3_index("acme");
    assert!(index.contains_key("api/v1"));
    assert!(index.contains_key("apis/apps/v1"));
    let url = index.get("api/v1").unwrap();
    assert!(
        url.contains("hash="),
        "openapi v3 index URL must carry a hash query param; got {url}"
    );
}

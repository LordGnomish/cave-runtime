// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Strict-TDD coverage for XRD→CRD generation (src/xrd/crd_gen.rs).
//!
//! Upstream: crossplane/crossplane-runtime pkg/xcrd
//!   - ForCompositeResource / ForCompositeResourceClaim (crd.go)
//!   - CompositeResourceSpecProps / CompositeResourceStatusProps (schemas.go)
//!
//! An XRD (CompositeResourceDefinition) is rendered into a Kubernetes
//! CustomResourceDefinition: names/scope/categories are derived and the
//! Crossplane *system fields* (compositionRef, compositionSelector,
//! compositionRevisionRef, compositionUpdatePolicy, resourceRefs, claimRef,
//! status.conditions, status.connectionDetails …) are injected into each
//! version's openAPIV3Schema. Pure in-crate transform — no apiserver.

use cave_crossplane::models::XrdScope;
use cave_crossplane::xrd::crd_gen::{
    for_composite_resource, for_composite_resource_claim, COMPOSITION_SPEC_FIELDS,
};
use cave_crossplane::xrd::spec::{XrdNames, XrdSpec, XrdSpecVersion};
use serde_json::json;

fn user_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "properties": {
            "spec": {
                "type": "object",
                "properties": { "size": {"type": "integer"} }
            }
        }
    })
}

fn db_xrd(scope: XrdScope) -> XrdSpec {
    let mut s = XrdSpec::new("example.org", "XDatabase", scope);
    s.versions = vec![XrdSpecVersion {
        name: "v1".into(),
        served: true,
        referenceable: true,
        schema: Some(user_schema()),
    }];
    s
}

#[test]
fn composition_spec_fields_constant() {
    assert!(COMPOSITION_SPEC_FIELDS.contains(&"compositionRef"));
    assert!(COMPOSITION_SPEC_FIELDS.contains(&"compositionSelector"));
    assert!(COMPOSITION_SPEC_FIELDS.contains(&"compositionRevisionRef"));
    assert!(COMPOSITION_SPEC_FIELDS.contains(&"compositionUpdatePolicy"));
}

#[test]
fn crd_names_scope_and_categories() {
    let crd = for_composite_resource(&db_xrd(XrdScope::Cluster));
    assert_eq!(crd["kind"], json!("CustomResourceDefinition"));
    assert_eq!(crd["metadata"]["name"], json!("xdatabases.example.org"));
    assert_eq!(crd["spec"]["group"], json!("example.org"));
    assert_eq!(crd["spec"]["scope"], json!("Cluster"));
    assert_eq!(crd["spec"]["names"]["kind"], json!("XDatabase"));
    assert_eq!(crd["spec"]["names"]["plural"], json!("xdatabases"));
    assert_eq!(crd["spec"]["names"]["listKind"], json!("XDatabaseList"));
    let cats = crd["spec"]["names"]["categories"].as_array().unwrap();
    assert!(cats.iter().any(|c| c == "composite"));
}

#[test]
fn injects_composition_system_spec_fields() {
    let crd = for_composite_resource(&db_xrd(XrdScope::Cluster));
    let spec_props =
        &crd["spec"]["versions"][0]["schema"]["openAPIV3Schema"]["properties"]["spec"]["properties"];
    for f in COMPOSITION_SPEC_FIELDS {
        assert!(
            spec_props.get(*f).is_some(),
            "expected injected spec field {f}, got {spec_props}"
        );
    }
    assert!(spec_props.get("resourceRefs").is_some());
    // compositionUpdatePolicy is a string enum.
    assert_eq!(spec_props["compositionUpdatePolicy"]["type"], json!("string"));
}

#[test]
fn injects_status_conditions() {
    let crd = for_composite_resource(&db_xrd(XrdScope::Cluster));
    let status_props = &crd["spec"]["versions"][0]["schema"]["openAPIV3Schema"]["properties"]
        ["status"]["properties"];
    assert!(status_props.get("conditions").is_some());
    assert_eq!(status_props["conditions"]["type"], json!("array"));
}

#[test]
fn cluster_scope_injects_claim_ref_and_connection_details() {
    let crd = for_composite_resource(&db_xrd(XrdScope::Cluster));
    let spec_props =
        &crd["spec"]["versions"][0]["schema"]["openAPIV3Schema"]["properties"]["spec"]["properties"];
    assert!(spec_props.get("claimRef").is_some());
    assert!(spec_props.get("writeConnectionSecretToRef").is_some());
    let status_props = &crd["spec"]["versions"][0]["schema"]["openAPIV3Schema"]["properties"]
        ["status"]["properties"];
    assert!(status_props.get("connectionDetails").is_some());
    assert!(
        status_props["connectionDetails"]["properties"]
            .get("lastPublishedTime")
            .is_some()
    );
}

#[test]
fn namespaced_scope_omits_claim_ref() {
    let crd = for_composite_resource(&db_xrd(XrdScope::Namespaced));
    assert_eq!(crd["spec"]["scope"], json!("Namespaced"));
    let spec_props =
        &crd["spec"]["versions"][0]["schema"]["openAPIV3Schema"]["properties"]["spec"]["properties"];
    assert!(
        spec_props.get("claimRef").is_none(),
        "namespaced XR must not carry claimRef"
    );
    // …but still carries the composition system fields + resourceRefs.
    assert!(spec_props.get("compositionRef").is_some());
    assert!(spec_props.get("resourceRefs").is_some());
}

#[test]
fn preserves_user_spec_fields() {
    let crd = for_composite_resource(&db_xrd(XrdScope::Cluster));
    let spec_props =
        &crd["spec"]["versions"][0]["schema"]["openAPIV3Schema"]["properties"]["spec"]["properties"];
    // The user-authored field survives the system-field injection.
    assert_eq!(spec_props["size"]["type"], json!("integer"));
}

#[test]
fn version_served_and_storage_flags() {
    let mut xrd = db_xrd(XrdScope::Cluster);
    xrd.versions.push(XrdSpecVersion {
        name: "v2".into(),
        served: true,
        referenceable: false,
        schema: Some(user_schema()),
    });
    let crd = for_composite_resource(&xrd);
    let versions = crd["spec"]["versions"].as_array().unwrap();
    assert_eq!(versions.len(), 2);
    // storage mirrors referenceable: exactly one storage=true.
    assert_eq!(versions[0]["storage"], json!(true));
    assert_eq!(versions[1]["storage"], json!(false));
    assert_eq!(versions[0]["served"], json!(true));
}

#[test]
fn claim_crd_none_without_claim_names() {
    let xrd = db_xrd(XrdScope::Cluster); // no claim_names
    assert!(for_composite_resource_claim(&xrd).is_none());
}

#[test]
fn claim_crd_names_scope_and_fields() {
    let xrd = db_xrd(XrdScope::Cluster).with_claim_names(XrdNames::from_kind("Database"));
    let crd = for_composite_resource_claim(&xrd).expect("claim CRD");
    assert_eq!(crd["metadata"]["name"], json!("databases.example.org"));
    // Claims are always namespaced.
    assert_eq!(crd["spec"]["scope"], json!("Namespaced"));
    let cats = crd["spec"]["names"]["categories"].as_array().unwrap();
    assert!(cats.iter().any(|c| c == "claim"));
    // Claim carries the composition system fields + conditions.
    let spec_props =
        &crd["spec"]["versions"][0]["schema"]["openAPIV3Schema"]["properties"]["spec"]["properties"];
    assert!(spec_props.get("compositionRef").is_some());
    let status_props = &crd["spec"]["versions"][0]["schema"]["openAPIV3Schema"]["properties"]
        ["status"]["properties"];
    assert!(status_props.get("conditions").is_some());
}

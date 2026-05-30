// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Strict-TDD RED → GREEN — `environment-configs` skip → mapped.
//!
//! Line-ports the in-memory EnvironmentConfig bag + environment field-path
//! patch resolution from crossplane v2.3.1:
//!   * apis/apiextensions/v1alpha1/environmentconfig_types.go (EnvironmentConfig.Data)
//!   * internal/controller/apiextensions/composite/environment/environment.go
//!     (buildEnvironment — ordered merge of selected EnvironmentConfigs into
//!      one in-memory `spec.environment` document)
//!   * internal/controller/apiextensions/composite/patches.go
//!     (PatchTypeFromEnvironmentFieldPath / PatchTypeToEnvironmentFieldPath)
//!
//! These are pure in-memory algorithms (key-value bag merge + dot-path patch)
//! with no apiserver / persistence dependency, hence honestly in-crate.

use cave_crossplane::environment::{Environment, EnvironmentConfig};
use cave_crossplane::models::{Patch, PatchType};

fn env_config(name: &str, data: &[(&str, serde_json::Value)]) -> EnvironmentConfig {
    let mut ec = EnvironmentConfig::new(name);
    for (k, v) in data {
        ec.set(k.to_string(), v.clone());
    }
    ec
}

fn from_env_patch(from: &str, to: &str) -> Patch {
    Patch {
        patch_type: PatchType::FromEnvironmentFieldPath,
        from_field_path: Some(from.to_string()),
        to_field_path: Some(to.to_string()),
        transforms: vec![],
        patch_set_name: None,
        combine: None,
    }
}

fn to_env_patch(from: &str, to: &str) -> Patch {
    Patch {
        patch_type: PatchType::ToEnvironmentFieldPath,
        from_field_path: Some(from.to_string()),
        to_field_path: Some(to.to_string()),
        transforms: vec![],
        patch_set_name: None,
        combine: None,
    }
}

// ── EnvironmentConfig key-value bag ────────────────────────────────────────────

#[test]
fn environment_config_data_bag_roundtrips() {
    let ec = env_config(
        "platform-defaults",
        &[
            ("region", serde_json::json!("eu-west-1")),
            ("size", serde_json::json!(3)),
        ],
    );
    assert_eq!(ec.name, "platform-defaults");
    assert_eq!(ec.get("region"), Some(&serde_json::json!("eu-west-1")));
    assert_eq!(ec.get("size"), Some(&serde_json::json!(3)));
    assert_eq!(ec.get("missing"), None);
}

// ── buildEnvironment: ordered merge (later selected configs win) ───────────────

#[test]
fn build_environment_merges_in_selection_order() {
    // environment.go buildEnvironment: configs merged in order, later overrides.
    let a = env_config(
        "base",
        &[
            ("region", serde_json::json!("us-east-1")),
            ("tier", serde_json::json!("standard")),
        ],
    );
    let b = env_config("override", &[("region", serde_json::json!("eu-west-1"))]);

    let env = Environment::build(&[a, b]);

    // `region` from the later config wins; `tier` from the earlier survives.
    assert_eq!(
        env.get_field_path("data.region"),
        Some(serde_json::json!("eu-west-1"))
    );
    assert_eq!(
        env.get_field_path("data.tier"),
        Some(serde_json::json!("standard"))
    );
}

#[test]
fn empty_environment_has_no_data() {
    let env = Environment::build(&[]);
    assert_eq!(env.get_field_path("data.anything"), None);
}

// ── FromEnvironmentFieldPath: env → composed resource ──────────────────────────

#[test]
fn from_environment_field_path_patches_composed_resource() {
    let env = Environment::build(&[env_config(
        "platform",
        &[("region", serde_json::json!("eu-west-1"))],
    )]);

    let mut composed = serde_json::json!({"spec": {}});
    let patch = from_env_patch("data.region", "spec.forProvider.region");

    let applied = env
        .apply_patch(&mut composed, &patch)
        .expect("from-env patch applies");
    assert!(applied, "patch with a present source must report applied=true");

    assert_eq!(
        composed["spec"]["forProvider"]["region"],
        serde_json::json!("eu-west-1")
    );
}

#[test]
fn from_environment_field_path_missing_source_is_noop() {
    // Upstream: a missing FromFieldPath source is skipped (not an error) unless
    // policy is Required; default policy Optional → no write, applied=false.
    let env = Environment::build(&[env_config("platform", &[])]);
    let mut composed = serde_json::json!({"spec": {}});
    let patch = from_env_patch("data.absent", "spec.forProvider.region");

    let applied = env.apply_patch(&mut composed, &patch).unwrap();
    assert!(!applied, "missing optional source must report applied=false");
    assert_eq!(composed, serde_json::json!({"spec": {}}));
}

// ── ToEnvironmentFieldPath: composed/composite → env ───────────────────────────

#[test]
fn to_environment_field_path_writes_back_into_environment() {
    let mut env = Environment::build(&[env_config("platform", &[])]);
    let source = serde_json::json!({"spec": {"parameters": {"region": "ap-south-1"}}});
    let patch = to_env_patch("spec.parameters.region", "data.region");

    let applied = env
        .apply_to_environment(&source, &patch)
        .expect("to-env patch applies");
    assert!(applied);

    assert_eq!(
        env.get_field_path("data.region"),
        Some(serde_json::json!("ap-south-1"))
    );
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Port-fidelity tests for KubeSchedulerConfiguration validation.
//!
//! Cite: kubernetes/kubernetes v1.36.0
//!   pkg/scheduler/apis/config/validation/validation.go
//!     ValidateKubeSchedulerConfiguration / validateKubeSchedulerProfile /
//!     validatePluginConfig / validatePercentageOfNodesToScore
//!
//! The config *types* are serde structs (models.rs), but the *validation*
//! rules — parallelism > 0, percentageOfNodesToScore ∈ [0,100], non-empty &
//! uniquely-named profiles, positive non-overflowing score weights, and
//! unique plugin-config names — are genuine algorithm, ported here.

use cave_scheduler::config_validation::{
    KubeSchedulerConfiguration, PluginConfig, PluginSet, ProfileConfig, WeightedPlugin,
};

fn valid_profile(name: &str) -> ProfileConfig {
    ProfileConfig {
        scheduler_name: name.to_string(),
        score: PluginSet::default(),
        plugin_config: vec![],
    }
}

fn valid_config() -> KubeSchedulerConfiguration {
    KubeSchedulerConfiguration {
        parallelism: 16,
        percentage_of_nodes_to_score: Some(0),
        profiles: vec![valid_profile("default-scheduler")],
    }
}

#[test]
fn a_well_formed_config_validates() {
    assert!(valid_config().validate().is_ok());
}

#[test]
fn parallelism_must_be_positive() {
    let mut c = valid_config();
    c.parallelism = 0;
    let errs = c.validate().unwrap_err();
    assert!(
        errs.iter().any(|e| e.contains("parallelism")),
        "expected a parallelism error, got {errs:?}"
    );
}

#[test]
fn percentage_of_nodes_must_be_in_range() {
    for bad in [-1, 101, 200] {
        let mut c = valid_config();
        c.percentage_of_nodes_to_score = Some(bad);
        let errs = c.validate().unwrap_err();
        assert!(
            errs.iter().any(|e| e.contains("percentageOfNodesToScore")),
            "expected range error for {bad}, got {errs:?}"
        );
    }
    // None (unset) and boundary values are accepted.
    let mut c = valid_config();
    c.percentage_of_nodes_to_score = None;
    assert!(c.validate().is_ok());
    c.percentage_of_nodes_to_score = Some(100);
    assert!(c.validate().is_ok());
}

#[test]
fn at_least_one_profile_required() {
    let mut c = valid_config();
    c.profiles.clear();
    let errs = c.validate().unwrap_err();
    assert!(errs.iter().any(|e| e.contains("profile")));
}

#[test]
fn each_profile_needs_a_scheduler_name() {
    let mut c = valid_config();
    c.profiles[0].scheduler_name = String::new();
    let errs = c.validate().unwrap_err();
    assert!(errs.iter().any(|e| e.contains("scheduler name")));
}

#[test]
fn scheduler_names_must_be_unique_across_profiles() {
    let mut c = valid_config();
    c.profiles.push(valid_profile("default-scheduler"));
    let errs = c.validate().unwrap_err();
    assert!(
        errs.iter().any(|e| e.contains("duplicate")
            && e.contains("default-scheduler")),
        "expected duplicate-name error, got {errs:?}"
    );
}

#[test]
fn score_weights_must_be_positive() {
    let mut c = valid_config();
    c.profiles[0].score.enabled.push(WeightedPlugin {
        name: "NodeResourcesFit".into(),
        weight: 0,
    });
    let errs = c.validate().unwrap_err();
    assert!(errs.iter().any(|e| e.contains("weight")));
}

#[test]
fn score_weights_must_not_overflow_total_score() {
    let mut c = valid_config();
    // MAX_NODE_SCORE (100) * weight must fit in i32.
    c.profiles[0].score.enabled.push(WeightedPlugin {
        name: "Huge".into(),
        weight: i32::MAX / 10,
    });
    let errs = c.validate().unwrap_err();
    assert!(
        errs.iter().any(|e| e.to_lowercase().contains("overflow")),
        "expected overflow error, got {errs:?}"
    );
}

#[test]
fn enabled_plugin_may_not_be_listed_twice() {
    let mut c = valid_config();
    c.profiles[0].score.enabled.push(WeightedPlugin {
        name: "Dup".into(),
        weight: 1,
    });
    c.profiles[0].score.enabled.push(WeightedPlugin {
        name: "Dup".into(),
        weight: 2,
    });
    let errs = c.validate().unwrap_err();
    assert!(errs.iter().any(|e| e.contains("Dup") && e.contains("enabled")));
}

#[test]
fn plugin_config_names_must_be_unique() {
    let mut c = valid_config();
    c.profiles[0].plugin_config = vec![
        PluginConfig { name: "X".into() },
        PluginConfig { name: "X".into() },
    ];
    let errs = c.validate().unwrap_err();
    assert!(errs.iter().any(|e| e.contains("plugin config") && e.contains("X")));
}

#[test]
fn all_independent_errors_are_aggregated() {
    let mut c = valid_config();
    c.parallelism = -1;
    c.percentage_of_nodes_to_score = Some(900);
    c.profiles[0].scheduler_name = String::new();
    let errs = c.validate().unwrap_err();
    // field.ErrorList semantics: collect every violation, not just the first.
    assert!(errs.len() >= 3, "expected >=3 aggregated errors, got {errs:?}");
}

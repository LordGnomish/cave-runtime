// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! The config-validation surface is reachable via the scheduler API
//! (`POST /api/scheduler/config/validate`). This tests the pure response
//! builder behind that route so the structured `{valid, errors}` contract is
//! pinned independent of the HTTP layer.

use cave_scheduler::config_validation::{KubeSchedulerConfiguration, ProfileConfig, PluginSet};
use cave_scheduler::routes::validate_config;

fn cfg(parallelism: i32) -> KubeSchedulerConfiguration {
    KubeSchedulerConfiguration {
        parallelism,
        percentage_of_nodes_to_score: Some(50),
        profiles: vec![ProfileConfig {
            scheduler_name: "default-scheduler".into(),
            score: PluginSet::default(),
            plugin_config: vec![],
        }],
    }
}

#[test]
fn valid_config_reports_ok_with_no_errors() {
    let resp = validate_config(&cfg(16));
    assert!(resp.valid);
    assert!(resp.errors.is_empty());
}

#[test]
fn invalid_config_reports_not_valid_with_errors() {
    let resp = validate_config(&cfg(0));
    assert!(!resp.valid);
    assert!(resp.errors.iter().any(|e| e.contains("parallelism")));
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! KubeSchedulerConfiguration validation.
//!
//! Cite: kubernetes/kubernetes v1.36.0
//!   pkg/scheduler/apis/config/validation/validation.go
//!     ValidateKubeSchedulerConfiguration, validateKubeSchedulerProfile,
//!     validatePluginConfig, validatePercentageOfNodesToScore
//!
//! The configuration *types* round-trip through serde (see `models.rs`); the
//! *validation* rules below are real algorithm. Like upstream's
//! `field.ErrorList`, [`KubeSchedulerConfiguration::validate`] does not stop at
//! the first violation — it accumulates every error so an operator sees the
//! whole set of problems at once.

use serde::{Deserialize, Serialize};

/// Maximum per-node score a plugin may emit, before its profile weight is
/// applied (upstream `framework.MaxNodeScore`).
pub const MAX_NODE_SCORE: i64 = 100;

/// A scoring plugin enabled in a profile, with its integer weight.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WeightedPlugin {
    pub name: String,
    pub weight: i32,
}

/// An enabled/disabled plugin set for one extension point. Only `enabled`
/// carries weights (the score extension point is the weighted one upstream).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PluginSet {
    pub enabled: Vec<WeightedPlugin>,
    pub disabled: Vec<String>,
}

/// Opaque per-plugin configuration entry; only its name is validated for
/// uniqueness here (args validation is plugin-specific upstream).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginConfig {
    pub name: String,
}

/// One scheduling profile's configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileConfig {
    pub scheduler_name: String,
    pub score: PluginSet,
    pub plugin_config: Vec<PluginConfig>,
}

/// Top-level scheduler configuration (subset of upstream
/// `KubeSchedulerConfiguration` carrying the fields with validation rules).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KubeSchedulerConfiguration {
    pub parallelism: i32,
    /// `None` means "unset → use the adaptive default"; when set it must be in
    /// `[0, 100]`.
    pub percentage_of_nodes_to_score: Option<i32>,
    pub profiles: Vec<ProfileConfig>,
}

impl KubeSchedulerConfiguration {
    /// Validate the whole configuration, returning every violation found
    /// (mirrors `ValidateKubeSchedulerConfiguration` aggregating a
    /// `field.ErrorList`). `Ok(())` iff the list is empty.
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errs = Vec::new();

        // validateParallelism: must be a positive value.
        if self.parallelism < 1 {
            errs.push(format!(
                "parallelism: should be a positive value, but got {}",
                self.parallelism
            ));
        }

        // validatePercentageOfNodesToScore: in range [0, 100] when set.
        if let Some(p) = self.percentage_of_nodes_to_score {
            if !(0..=100).contains(&p) {
                errs.push(format!(
                    "percentageOfNodesToScore: not in valid range [0, 100], got {p}"
                ));
            }
        }

        // At least one profile must be specified.
        if self.profiles.is_empty() {
            errs.push("profiles: must specify at least one scheduling profile".to_string());
        }

        // Scheduler names must be unique across profiles.
        let mut seen_names: Vec<&str> = Vec::new();
        for (i, prof) in self.profiles.iter().enumerate() {
            if prof.scheduler_name.is_empty() {
                errs.push(format!(
                    "profiles[{i}]: scheduler name is required for each profile"
                ));
            } else if seen_names.contains(&prof.scheduler_name.as_str()) {
                errs.push(format!(
                    "profiles[{i}]: duplicate profile with scheduler name '{}'",
                    prof.scheduler_name
                ));
            } else {
                seen_names.push(&prof.scheduler_name);
            }

            validate_score_weights(&prof.score, i, &mut errs);
            validate_enabled_uniqueness(&prof.score, i, &mut errs);
            validate_plugin_config_uniqueness(&prof.plugin_config, i, &mut errs);
        }

        if errs.is_empty() {
            Ok(())
        } else {
            Err(errs)
        }
    }
}

/// Each enabled score plugin's weight must be positive and `MAX_NODE_SCORE *
/// weight` must not overflow `i32` (upstream rejects weights whose total score
/// would overflow the int32 score accumulator).
fn validate_score_weights(set: &PluginSet, profile_idx: usize, errs: &mut Vec<String>) {
    for p in &set.enabled {
        if p.weight <= 0 {
            errs.push(format!(
                "profiles[{profile_idx}].plugins.score: plugin '{}' weight must be a positive value, got {}",
                p.name, p.weight
            ));
            continue;
        }
        if MAX_NODE_SCORE.saturating_mul(p.weight as i64) > i32::MAX as i64 {
            errs.push(format!(
                "profiles[{profile_idx}].plugins.score: total score of plugin '{}' would overflow",
                p.name
            ));
        }
    }
}

/// A plugin name may not appear more than once in the enabled set
/// (upstream `validatePluginSetForInvalidPlugins` / duplicate detection).
fn validate_enabled_uniqueness(set: &PluginSet, profile_idx: usize, errs: &mut Vec<String>) {
    let mut seen: Vec<&str> = Vec::new();
    for p in &set.enabled {
        if seen.contains(&p.name.as_str()) {
            errs.push(format!(
                "profiles[{profile_idx}].plugins.score: plugin '{}' already registered as enabled",
                p.name
            ));
        } else {
            seen.push(&p.name);
        }
    }
}

/// Plugin config names must be unique within a profile
/// (upstream `validatePluginConfig`: "duplicated").
fn validate_plugin_config_uniqueness(
    cfgs: &[PluginConfig],
    profile_idx: usize,
    errs: &mut Vec<String>,
) {
    let mut seen: Vec<&str> = Vec::new();
    for c in cfgs {
        if seen.contains(&c.name.as_str()) {
            errs.push(format!(
                "profiles[{profile_idx}].pluginConfig: duplicated plugin config '{}'",
                c.name
            ));
        } else {
            seen.push(&c.name);
        }
    }
}

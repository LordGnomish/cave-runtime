//! Intent parsing, dependency resolution, validation, and state diffing.

use crate::models::{DriftItem, DriftReport, InfraIntent, InfraResource, InfraState};
use anyhow::{bail, Result};
use std::collections::{HashMap, HashSet, VecDeque};
use uuid::Uuid;

/// Parse a raw string into an `InfraIntent`.
///
/// Accepts either:
/// - Plain natural language  → stored in `natural_language`
/// - YAML/JSON blob          → parsed into `structured`
pub fn parse_intent(
    raw: &str,
    name: impl Into<String>,
    environment: impl Into<String>,
) -> Result<InfraIntent> {
    let trimmed = raw.trim();
    let mut intent = InfraIntent::new(name, environment);

    // Try YAML first (YAML is a superset of JSON).
    match serde_yaml::from_str::<serde_json::Value>(trimmed) {
        Ok(val) if val.is_object() || val.is_array() => {
            intent.structured = Some(val);
        }
        _ => {
            // Treat as natural language.
            if trimmed.is_empty() {
                bail!("intent text is empty");
            }
            intent.natural_language = Some(trimmed.to_string());
        }
    }

    Ok(intent)
}

/// Validate an intent for obvious problems.
pub fn validate_intent(intent: &InfraIntent) -> Result<Vec<String>> {
    let mut warnings = Vec::new();

    if intent.natural_language.is_none() && intent.structured.is_none() {
        bail!("intent has neither natural_language nor structured content");
    }

    if intent.environment.is_empty() {
        bail!("intent environment must not be empty");
    }

    if intent.environment == "prod" || intent.environment == "production" {
        warnings.push("targeting production environment — apply with caution".into());
    }

    if let Some(ref s) = intent.structured {
        if let Some(obj) = s.as_object() {
            if !obj.contains_key("resources") && !obj.contains_key("resource") {
                warnings.push(
                    "structured intent has no 'resources' key — may produce an empty plan".into(),
                );
            }
        }
    }

    Ok(warnings)
}

/// Topologically sort resources respecting their `dependencies` field.
///
/// Returns resources in execution order (dependencies first).
/// Returns an error if a cycle is detected.
pub fn resolve_dependencies(resources: &[InfraResource]) -> Result<Vec<InfraResource>> {
    let by_id: HashMap<Uuid, &InfraResource> =
        resources.iter().map(|r| (r.id, r)).collect();

    // Kahn's algorithm.
    let mut in_degree: HashMap<Uuid, usize> = resources.iter().map(|r| (r.id, 0)).collect();
    let mut adj: HashMap<Uuid, Vec<Uuid>> = resources.iter().map(|r| (r.id, vec![])).collect();

    for r in resources {
        for &dep in &r.dependencies {
            if !by_id.contains_key(&dep) {
                bail!(
                    "resource '{}' depends on unknown resource id {}",
                    r.name,
                    dep
                );
            }
            // dep → r  (r cannot run before dep)
            adj.entry(dep).or_default().push(r.id);
            *in_degree.entry(r.id).or_default() += 1;
        }
    }

    let mut queue: VecDeque<Uuid> = in_degree
        .iter()
        .filter_map(|(&id, &deg)| if deg == 0 { Some(id) } else { None })
        .collect();

    let mut ordered = Vec::with_capacity(resources.len());
    let mut visited = HashSet::new();

    while let Some(id) = queue.pop_front() {
        if visited.contains(&id) {
            continue;
        }
        visited.insert(id);
        ordered.push((*by_id[&id]).clone());

        for &next in adj.get(&id).into_iter().flatten() {
            let deg = in_degree.entry(next).or_default();
            *deg = deg.saturating_sub(1);
            if *deg == 0 {
                queue.push_back(next);
            }
        }
    }

    if ordered.len() != resources.len() {
        bail!("dependency cycle detected among resources");
    }

    Ok(ordered)
}

/// Compute the diff between current and desired state.
///
/// Returns a tuple of `(to_create, to_update, to_delete)` resource lists.
pub fn diff_state(state: &InfraState) -> (Vec<InfraResource>, Vec<InfraResource>, Vec<InfraResource>) {
    let actual_by_name: HashMap<&str, &InfraResource> =
        state.actual.iter().map(|r| (r.name.as_str(), r)).collect();
    let desired_by_name: HashMap<&str, &InfraResource> =
        state.desired.iter().map(|r| (r.name.as_str(), r)).collect();

    let mut to_create = Vec::new();
    let mut to_update = Vec::new();
    let mut to_delete = Vec::new();

    for desired in &state.desired {
        match actual_by_name.get(desired.name.as_str()) {
            None => to_create.push(desired.clone()),
            Some(actual) => {
                if needs_update(desired, actual) {
                    to_update.push(desired.clone());
                }
            }
        }
    }

    for actual in &state.actual {
        if !desired_by_name.contains_key(actual.name.as_str()) {
            to_delete.push(actual.clone());
        }
    }

    (to_create, to_update, to_delete)
}

/// Determine whether a resource needs an update by comparing configs.
fn needs_update(desired: &InfraResource, actual: &InfraResource) -> bool {
    if desired.resource_type != actual.resource_type || desired.provider != actual.provider {
        return true;
    }
    // Deep compare config via JSON serialization.
    let d = serde_json::to_string(&desired.config).unwrap_or_default();
    let a = serde_json::to_string(&actual.config).unwrap_or_default();
    d != a
}

/// Build a `DriftReport` by comparing desired vs actual state.
pub fn detect_drift(state: &InfraState) -> DriftReport {
    let mut report = DriftReport::new();

    let actual_by_name: HashMap<&str, &InfraResource> =
        state.actual.iter().map(|r| (r.name.as_str(), r)).collect();
    let desired_by_name: HashMap<&str, &InfraResource> =
        state.desired.iter().map(|r| (r.name.as_str(), r)).collect();

    for desired in &state.desired {
        match actual_by_name.get(desired.name.as_str()) {
            None => {
                report.missing.push(desired.name.clone());
            }
            Some(actual) => {
                let drifted_fields = find_drifted_fields(desired, actual);
                if !drifted_fields.is_empty() {
                    report.drifted.push(DriftItem {
                        resource_id: desired.id,
                        resource_name: desired.name.clone(),
                        provider: desired.provider.clone(),
                        resource_type: desired.resource_type.clone(),
                        drifted_fields,
                        desired: serde_json::to_value(&desired.config)
                            .unwrap_or(serde_json::Value::Null),
                        actual: serde_json::to_value(&actual.config)
                            .unwrap_or(serde_json::Value::Null),
                    });
                }
            }
        }
    }

    for actual in &state.actual {
        if !desired_by_name.contains_key(actual.name.as_str()) {
            report.orphaned.push(actual.name.clone());
        }
    }

    report
}

/// Return field names whose values differ between desired and actual configs.
fn find_drifted_fields(desired: &InfraResource, actual: &InfraResource) -> Vec<String> {
    let mut drifted = Vec::new();

    for (key, desired_val) in &desired.config {
        match actual.config.get(key) {
            None => drifted.push(key.clone()),
            Some(actual_val) => {
                if desired_val != actual_val {
                    drifted.push(key.clone());
                }
            }
        }
    }

    // Keys present in actual but absent in desired are also drift.
    for key in actual.config.keys() {
        if !desired.config.contains_key(key) {
            drifted.push(key.clone());
        }
    }

    drifted
}

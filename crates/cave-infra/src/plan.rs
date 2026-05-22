// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Change plan engine — diff desired vs actual state, generate an execution plan.

use crate::error::{InfraError, InfraResult};
use crate::graph::apply_order;
use crate::resource::{ResourceSpec, ResourceState, ResourceStatus, ResourceStore};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ── Plan operation ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum PlanOp {
    Create,
    Update,
    Delete,
    NoChange,
    Recreate,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedChange {
    pub op: PlanOp,
    pub resource_kind: String,
    pub resource_name: String,
    pub reason: String,
    pub diff: Vec<FieldDiff>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldDiff {
    pub field: String,
    pub before: Option<serde_json::Value>,
    pub after: Option<serde_json::Value>,
}

// ── Plan ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub id: Uuid,
    pub changes: Vec<PlannedChange>,
    pub created_at: DateTime<Utc>,
    pub approved: bool,
    pub summary: PlanSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanSummary {
    pub to_create: usize,
    pub to_update: usize,
    pub to_delete: usize,
    pub no_change: usize,
    pub to_recreate: usize,
}

impl Plan {
    pub fn new(changes: Vec<PlannedChange>) -> Self {
        let summary = PlanSummary {
            to_create: changes.iter().filter(|c| c.op == PlanOp::Create).count(),
            to_update: changes.iter().filter(|c| c.op == PlanOp::Update).count(),
            to_delete: changes.iter().filter(|c| c.op == PlanOp::Delete).count(),
            no_change: changes.iter().filter(|c| c.op == PlanOp::NoChange).count(),
            to_recreate: changes.iter().filter(|c| c.op == PlanOp::Recreate).count(),
        };
        Self {
            id: Uuid::new_v4(),
            changes,
            created_at: Utc::now(),
            approved: false,
            summary,
        }
    }

    pub fn has_changes(&self) -> bool {
        self.summary.to_create > 0
            || self.summary.to_update > 0
            || self.summary.to_delete > 0
            || self.summary.to_recreate > 0
    }
}

// ── Plan engine ───────────────────────────────────────────────────────────────

/// Generate a plan by diffing desired specs against the current state store.
pub fn generate_plan(desired: &[ResourceSpec], store: &ResourceStore) -> InfraResult<Plan> {
    // Validate dependencies first
    crate::graph::validate_dependencies(desired)?;

    let mut changes = Vec::new();

    // Check what needs to be created or updated
    for spec in desired {
        let key = format!("{}/{}", spec.kind.as_str(), spec.name);
        match store.get(&key) {
            Err(_) => {
                // Resource doesn't exist → Create
                changes.push(PlannedChange {
                    op: PlanOp::Create,
                    resource_kind: spec.kind.as_str(),
                    resource_name: spec.name.clone(),
                    reason: "new resource".into(),
                    diff: spec
                        .properties
                        .iter()
                        .map(|(k, v)| FieldDiff {
                            field: k.clone(),
                            before: None,
                            after: Some(v.clone()),
                        })
                        .collect(),
                });
            }
            Ok(current) => {
                let drifts = diff_specs(&current.spec, spec);
                if drifts.is_empty() {
                    changes.push(PlannedChange {
                        op: PlanOp::NoChange,
                        resource_kind: spec.kind.as_str(),
                        resource_name: spec.name.clone(),
                        reason: "no changes".into(),
                        diff: vec![],
                    });
                } else {
                    changes.push(PlannedChange {
                        op: PlanOp::Update,
                        resource_kind: spec.kind.as_str(),
                        resource_name: spec.name.clone(),
                        reason: format!("{} field(s) changed", drifts.len()),
                        diff: drifts,
                    });
                }
            }
        }
    }

    // Check what needs to be deleted (in store but not in desired)
    let desired_keys: std::collections::HashSet<String> = desired
        .iter()
        .map(|s| format!("{}/{}", s.kind.as_str(), s.name))
        .collect();
    for state in store.list() {
        if !desired_keys.contains(&state.key()) && state.status != ResourceStatus::Deleted {
            changes.push(PlannedChange {
                op: PlanOp::Delete,
                resource_kind: state.spec.kind.as_str(),
                resource_name: state.spec.name.clone(),
                reason: "not in desired state".into(),
                diff: state
                    .spec
                    .properties
                    .iter()
                    .map(|(k, v)| FieldDiff {
                        field: k.clone(),
                        before: Some(v.clone()),
                        after: None,
                    })
                    .collect(),
            });
        }
    }

    Ok(Plan::new(changes))
}

fn diff_specs(current: &ResourceSpec, desired: &ResourceSpec) -> Vec<FieldDiff> {
    let mut diffs = Vec::new();

    // Fields added or changed in desired
    for (key, desired_val) in &desired.properties {
        match current.properties.get(key) {
            None => diffs.push(FieldDiff {
                field: key.clone(),
                before: None,
                after: Some(desired_val.clone()),
            }),
            Some(cur_val) if cur_val != desired_val => diffs.push(FieldDiff {
                field: key.clone(),
                before: Some(cur_val.clone()),
                after: Some(desired_val.clone()),
            }),
            _ => {}
        }
    }

    // Fields removed in desired
    for key in current.properties.keys() {
        if !desired.properties.contains_key(key) {
            diffs.push(FieldDiff {
                field: key.clone(),
                before: current.properties.get(key).cloned(),
                after: None,
            });
        }
    }

    diffs
}

/// Execution record for a completed plan application.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyRecord {
    pub plan_id: Uuid,
    pub applied_at: DateTime<Utc>,
    pub succeeded: Vec<String>,
    pub failed: Vec<(String, String)>,
    pub duration_ms: i64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resource::{ResourceKind, ResourceState};
    use std::collections::HashMap;

    fn spec(name: &str, props: Vec<(&str, serde_json::Value)>) -> ResourceSpec {
        ResourceSpec {
            kind: ResourceKind::Server,
            name: name.to_string(),
            provider: "noop".into(),
            properties: props.into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
            depends_on: vec![],
            tags: HashMap::new(),
        }
    }

    #[test]
    fn plan_creates_new_resource() {
        let store = ResourceStore::new();
        let desired = vec![spec("web-01", vec![("cpu", serde_json::json!(4))])];
        let plan = generate_plan(&desired, &store).unwrap();
        assert_eq!(plan.summary.to_create, 1);
        assert_eq!(plan.summary.to_update, 0);
    }

    #[test]
    fn plan_no_change_when_identical() {
        let store = ResourceStore::new();
        let spec1 = spec("db-01", vec![("cpu", serde_json::json!(8))]);
        let mut state = ResourceState::new(spec1.clone());
        state.transition(ResourceStatus::Running);
        store.upsert(state);

        let desired = vec![spec1];
        let plan = generate_plan(&desired, &store).unwrap();
        assert_eq!(plan.summary.no_change, 1);
        assert_eq!(plan.summary.to_update, 0);
        assert!(!plan.has_changes());
    }

    #[test]
    fn plan_updates_changed_resource() {
        let store = ResourceStore::new();
        let original = spec("lb-01", vec![("cpu", serde_json::json!(2))]);
        let mut state = ResourceState::new(original);
        state.transition(ResourceStatus::Running);
        store.upsert(state);

        let desired = vec![spec("lb-01", vec![("cpu", serde_json::json!(4))])];
        let plan = generate_plan(&desired, &store).unwrap();
        assert_eq!(plan.summary.to_update, 1);
        let change = &plan.changes[0];
        assert_eq!(change.diff[0].field, "cpu");
        assert_eq!(change.diff[0].before, Some(serde_json::json!(2)));
        assert_eq!(change.diff[0].after, Some(serde_json::json!(4)));
    }

    #[test]
    fn plan_deletes_removed_resource() {
        let store = ResourceStore::new();
        let s = spec("old-server", vec![]);
        let mut state = ResourceState::new(s);
        state.transition(ResourceStatus::Running);
        store.upsert(state);

        // desired is empty
        let plan = generate_plan(&[], &store).unwrap();
        assert_eq!(plan.summary.to_delete, 1);
    }
}

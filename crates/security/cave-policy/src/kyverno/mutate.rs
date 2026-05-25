// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Kyverno mutation engine.
//!
//! Supports: patchStrategicMerge, patchesJson6902, foreach.

use super::jmespath::{evaluate, substitute_variables_json};
use super::models::*;
use crate::error::PolicyError;
use crate::rego::value::apply_json_patch;
use serde_json::Value;

/// Apply a Kyverno mutate rule to a resource.
/// Returns the mutated resource and a list of JSON patch operations (for admission response).
pub fn mutate_rule(
    rule: &KyvernoRule,
    resource: &Value,
    context: &Value,
) -> Result<Option<(Value, Vec<serde_json::Value>)>, PolicyError> {
    let mutate = match &rule.mutate {
        Some(m) => m,
        None => return Ok(None),
    };

    // Evaluate preconditions
    if let Some(preconditions) = &rule.preconditions {
        if !super::validate::eval_conditions(preconditions, resource, context)? {
            return Ok(None);
        }
    }

    let mut current = resource.clone();
    let mut patches: Vec<serde_json::Value> = Vec::new();

    // patchStrategicMerge
    if let Some(psm) = &mutate.patch_strategic_merge {
        let substituted = substitute_variables_json(psm, context)?;
        let (new_resource, new_patches) = apply_strategic_merge(&current, &substituted)?;
        patches.extend(new_patches);
        current = new_resource;
    }

    // patchesJson6902
    if let Some(patches_str) = &mutate.patches_json6902 {
        let substituted_str = super::jmespath::substitute_variables(patches_str, context)?;
        let ops: Vec<crate::models::JsonPatchOp> = serde_yaml::from_str(&substituted_str)
            .or_else(|_| serde_json::from_str::<Vec<crate::models::JsonPatchOp>>(&substituted_str))
            .map_err(|e| PolicyError::Mutation(format!("invalid patchesJson6902: {e}")))?;
        for op in &ops {
            apply_json_patch(
                &mut current,
                &op.op,
                &op.path,
                op.value.as_ref(),
                op.from.as_deref(),
            )
            .map_err(|e| PolicyError::Mutation(e))?;
            patches.push(serde_json::to_value(op).unwrap_or_default());
        }
    }

    // foreach mutations
    if !mutate.foreach.is_empty() {
        for foreach in &mutate.foreach {
            let (new_resource, new_patches) = mutate_foreach(foreach, &current, context)?;
            patches.extend(new_patches);
            current = new_resource;
        }
    }

    if patches.is_empty() {
        Ok(None)
    } else {
        Ok(Some((current, patches)))
    }
}

fn mutate_foreach(
    foreach: &ForEachMutation,
    resource: &Value,
    context: &Value,
) -> Result<(Value, Vec<serde_json::Value>), PolicyError> {
    let list = evaluate(&foreach.list, resource)?;
    let items = match &list {
        Value::Array(a) => a.clone(),
        _ => return Ok((resource.clone(), vec![])),
    };

    let mut current = resource.clone();
    let mut patches = Vec::new();

    for item in &items {
        // Check preconditions
        if let Some(preconds) = &foreach.preconditions {
            if !super::validate::eval_conditions(preconds, item, context)? {
                continue;
            }
        }

        // patchStrategicMerge
        if let Some(psm) = &foreach.patch_strategic_merge {
            let substituted = substitute_variables_json(psm, context)?;
            let (new_resource, new_patches) = apply_strategic_merge(&current, &substituted)?;
            patches.extend(new_patches);
            current = new_resource;
        }

        // patchesJson6902
        if let Some(patches_str) = &foreach.patches_json6902 {
            let substituted_str = super::jmespath::substitute_variables(patches_str, context)?;
            let ops: Vec<crate::models::JsonPatchOp> = serde_yaml::from_str(&substituted_str)
                .or_else(|_| {
                    serde_json::from_str::<Vec<crate::models::JsonPatchOp>>(&substituted_str)
                })
                .map_err(|e| PolicyError::Mutation(format!("invalid patchesJson6902: {e}")))?;
            for op in &ops {
                apply_json_patch(
                    &mut current,
                    &op.op,
                    &op.path,
                    op.value.as_ref(),
                    op.from.as_deref(),
                )
                .map_err(|e| PolicyError::Mutation(e))?;
                patches.push(serde_json::to_value(op).unwrap_or_default());
            }
        }
    }

    Ok((current, patches))
}

/// Apply a strategic merge patch to a JSON document.
/// Returns the merged document and a list of JSON Patch operations representing the diff.
pub fn apply_strategic_merge(
    base: &Value,
    patch: &Value,
) -> Result<(Value, Vec<serde_json::Value>), PolicyError> {
    let mut result = base.clone();
    let mut ops = Vec::new();
    strategic_merge_recursive(&mut result, patch, "/", &mut ops);
    Ok((result, ops))
}

fn strategic_merge_recursive(
    target: &mut Value,
    patch: &Value,
    path: &str,
    ops: &mut Vec<serde_json::Value>,
) {
    match (target, patch) {
        (Value::Object(t), Value::Object(p)) => {
            for (key, pval) in p {
                // Handle strategic merge directives
                if key == "$patch" || key.starts_with("(") {
                    continue; // Skip directive keys
                }

                let child_path = if path == "/" {
                    format!("/{}", json_pointer_escape(key))
                } else {
                    format!("{}/{}", path, json_pointer_escape(key))
                };

                if let Some(tval) = t.get_mut(key) {
                    match (tval, pval) {
                        (tv @ Value::Object(_), pv @ Value::Object(_)) => {
                            strategic_merge_recursive(tv, pv, &child_path, ops);
                        }
                        (tv, pv) if tv == pv => {} // No change
                        (tv, pv) => {
                            // Replace
                            ops.push(serde_json::json!({
                                "op": "replace",
                                "path": child_path,
                                "value": pv
                            }));
                            *tv = pv.clone();
                        }
                    }
                } else {
                    // Add new field
                    ops.push(serde_json::json!({
                        "op": "add",
                        "path": child_path,
                        "value": pval
                    }));
                    t.insert(key.clone(), pval.clone());
                }
            }
        }
        (Value::Array(t), Value::Array(p)) => {
            // Merge arrays: append new items
            let existing_len = t.len();
            for (i, pitem) in p.iter().enumerate() {
                if i < existing_len {
                    if t[i] != *pitem {
                        ops.push(serde_json::json!({
                            "op": "replace",
                            "path": format!("{}/{}", path, i),
                            "value": pitem
                        }));
                        t[i] = pitem.clone();
                    }
                } else {
                    ops.push(serde_json::json!({
                        "op": "add",
                        "path": format!("{}/-", path),
                        "value": pitem
                    }));
                    t.push(pitem.clone());
                }
            }
        }
        (t, p) => {
            if t != p {
                ops.push(serde_json::json!({
                    "op": "replace",
                    "path": path,
                    "value": p
                }));
                *t = p.clone();
            }
        }
    }
}

fn json_pointer_escape(s: &str) -> String {
    s.replace('~', "~0").replace('/', "~1")
}

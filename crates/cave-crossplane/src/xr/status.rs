// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! XR status reconciliation — aggregates composed-resource readiness,
//! emits composition revision + connection details + composed-resource refs
//! into the XR status sub-resource.
//!
//! Upstream: internal/controller/apiextensions/composite/composed.go::UpdateXRStatus

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct XrStatusSummary {
    pub ready: bool,
    pub synced: bool,
    pub composition_revision: Option<String>,
    pub connection_secret_ref: Option<String>,
    pub composed_refs: Vec<ComposedRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ComposedRef {
    pub api_version: String,
    pub kind: String,
    pub name: String,
}

/// Aggregate a list of composed-resource JSON blobs and return the subset
/// that have `Ready=True` in `status.conditions`.
pub fn aggregate_ready(composed: &[Value]) -> Vec<&Value> {
    composed
        .iter()
        .filter(|v| has_condition_true(v, "Ready"))
        .collect()
}

pub fn has_condition_true(resource: &Value, ctype: &str) -> bool {
    resource
        .get("status")
        .and_then(|s| s.get("conditions"))
        .and_then(|c| c.as_array())
        .map(|arr| {
            arr.iter().any(|cond| {
                cond.get("type").and_then(|v| v.as_str()) == Some(ctype)
                    && cond.get("status").and_then(|v| v.as_str()) == Some("True")
            })
        })
        .unwrap_or(false)
}

/// Build a status summary from the composed-resource set.
pub fn summarize(composed: &[Value], composition_revision: Option<&str>) -> XrStatusSummary {
    let total = composed.len();
    let ready_count = aggregate_ready(composed).len();
    let synced_count = composed.iter().filter(|v| has_condition_true(v, "Synced")).count();

    let refs: Vec<ComposedRef> = composed
        .iter()
        .filter_map(|v| {
            Some(ComposedRef {
                api_version: v.get("apiVersion")?.as_str()?.to_string(),
                kind: v.get("kind")?.as_str()?.to_string(),
                name: v.get("metadata")?.get("name")?.as_str()?.to_string(),
            })
        })
        .collect();

    XrStatusSummary {
        ready: total > 0 && ready_count == total,
        synced: total > 0 && synced_count == total,
        composition_revision: composition_revision.map(|s| s.to_string()),
        connection_secret_ref: None,
        composed_refs: refs,
    }
}

/// Merge `summary` into `xr.status` returning a new XR JSON value.
pub fn write_status(xr: &Value, summary: &XrStatusSummary) -> Value {
    let mut xr = xr.clone();
    let status_obj = xr
        .as_object_mut()
        .and_then(|o| {
            o.entry("status".to_string())
                .or_insert(Value::Object(Map::new()))
                .as_object_mut()
        })
        .map(|o| o.to_owned());
    let mut new_status = status_obj.unwrap_or_default();
    new_status.insert(
        "ready".to_string(),
        Value::Bool(summary.ready),
    );
    new_status.insert(
        "synced".to_string(),
        Value::Bool(summary.synced),
    );
    if let Some(rev) = &summary.composition_revision {
        new_status.insert(
            "compositionRevisionRef".to_string(),
            serde_json::json!({"name": rev}),
        );
    }
    new_status.insert(
        "composedResourceRefs".to_string(),
        serde_json::to_value(&summary.composed_refs).unwrap_or(Value::Null),
    );
    if let Some(o) = xr.as_object_mut() {
        o.insert("status".to_string(), Value::Object(new_status));
    }
    xr
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn composed(name: &str, conds: &[(&str, &str)]) -> Value {
        let mut conditions = Vec::new();
        for (t, s) in conds {
            conditions.push(json!({"type": t, "status": s}));
        }
        json!({
            "apiVersion":"ex.cave.io/v1",
            "kind":"Bucket",
            "metadata":{"name":name},
            "status":{"conditions": conditions}
        })
    }

    #[test]
    fn empty_composed_not_ready() {
        assert!(aggregate_ready(&[]).is_empty());
        assert!(!summarize(&[], None).ready);
    }

    #[test]
    fn all_ready_summary_ready() {
        let c = vec![
            composed("a", &[("Ready", "True")]),
            composed("b", &[("Ready", "True")]),
        ];
        let s = summarize(&c, Some("rev-1"));
        assert!(s.ready);
        assert_eq!(s.composition_revision.as_deref(), Some("rev-1"));
        assert_eq!(s.composed_refs.len(), 2);
    }

    #[test]
    fn partial_ready_not_ready_summary() {
        let c = vec![
            composed("a", &[("Ready", "True")]),
            composed("b", &[("Ready", "False")]),
        ];
        let s = summarize(&c, None);
        assert!(!s.ready);
    }

    #[test]
    fn synced_aggregation() {
        let c = vec![composed("a", &[("Synced", "True")])];
        let s = summarize(&c, None);
        assert!(s.synced);
    }

    #[test]
    fn write_status_merges() {
        let xr = json!({"metadata":{"name":"x"}});
        let s = XrStatusSummary {
            ready: true,
            synced: true,
            composition_revision: Some("r1".into()),
            connection_secret_ref: None,
            composed_refs: vec![],
        };
        let merged = write_status(&xr, &s);
        assert_eq!(merged["status"]["ready"], json!(true));
        assert_eq!(
            merged["status"]["compositionRevisionRef"]["name"],
            json!("r1")
        );
    }

    #[test]
    fn has_condition_true_negative() {
        let r = json!({"status":{"conditions":[{"type":"Ready","status":"False"}]}});
        assert!(!has_condition_true(&r, "Ready"));
    }

    #[test]
    fn aggregate_ready_filters() {
        let c = vec![
            composed("a", &[("Ready", "True")]),
            composed("b", &[("Ready", "False")]),
        ];
        let ready = aggregate_ready(&c);
        assert_eq!(ready.len(), 1);
    }
}

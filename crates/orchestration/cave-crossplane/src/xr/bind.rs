// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Claim ↔ XR binder.
//!
//! Upstream: internal/controller/apiextensions/claim/binder.go
//!
//! A namespace-scoped Claim and a cluster-scoped XR are bound by two-way
//! references: the XR carries `spec.claimRef` (namespace + name), and the
//! Claim carries `spec.resourceRef` (name of the XR). Spec values are merged
//! from claim → XR (XR keeps `claimRef`, drops to claim spec for the rest).

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClaimRefJson {
    pub namespace: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResourceRef {
    pub name: String,
}

/// Bind a claim to an XR — returns updated (claim, xr) pair with cross-refs set.
pub fn bind_claim_to_xr(claim: &Value, xr: &Value) -> (Value, Value) {
    let claim_name = claim
        .get("metadata")
        .and_then(|m| m.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let claim_ns = claim
        .get("metadata")
        .and_then(|m| m.get("namespace"))
        .and_then(|v| v.as_str())
        .unwrap_or("default")
        .to_string();
    let xr_name = xr
        .get("metadata")
        .and_then(|m| m.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let mut claim = claim.clone();
    let mut xr = xr.clone();

    // Stamp xr.spec.claimRef
    set_path(
        &mut xr,
        &["spec", "claimRef"],
        serde_json::json!({"namespace": claim_ns, "name": claim_name}),
    );

    // Stamp claim.spec.resourceRef
    set_path(
        &mut claim,
        &["spec", "resourceRef"],
        serde_json::json!({"name": xr_name}),
    );

    (claim, xr)
}

/// Apply defaults from XRD to a claim's spec — fills any field in `defaults`
/// that's absent from the claim spec.
pub fn default_claim_from_xr_spec(claim: &Value, defaults: &Value) -> Value {
    let mut claim = claim.clone();
    let mut spec = claim
        .as_object()
        .and_then(|o| o.get("spec"))
        .cloned()
        .unwrap_or(Value::Object(Map::new()));
    if let (Some(sp), Some(dp)) = (spec.as_object_mut(), defaults.as_object()) {
        for (k, v) in dp {
            sp.entry(k.clone()).or_insert(v.clone());
        }
    }
    if let Some(o) = claim.as_object_mut() {
        o.insert("spec".to_string(), spec);
    }
    claim
}

/// Returns true iff the resource is namespace-scoped per metadata.namespace.
pub fn is_namespace_scoped(resource: &Value) -> bool {
    resource
        .get("metadata")
        .and_then(|m| m.get("namespace"))
        .and_then(|v| v.as_str())
        .is_some()
}

fn set_path(v: &mut Value, path: &[&str], value: Value) {
    let mut cur = v;
    for (i, seg) in path.iter().enumerate() {
        if i == path.len() - 1 {
            if let Some(o) = cur.as_object_mut() {
                o.insert(seg.to_string(), value);
                return;
            }
        } else {
            if !cur.is_object() {
                *cur = Value::Object(Map::new());
            }
            let map = cur.as_object_mut().unwrap();
            let entry = map
                .entry(seg.to_string())
                .or_insert(Value::Object(Map::new()));
            cur = entry;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn binding_sets_cross_refs() {
        let claim = json!({"metadata":{"namespace":"ns","name":"c1"}});
        let xr = json!({"metadata":{"name":"x1"}});
        let (c, x) = bind_claim_to_xr(&claim, &xr);
        assert_eq!(x["spec"]["claimRef"]["name"], json!("c1"));
        assert_eq!(x["spec"]["claimRef"]["namespace"], json!("ns"));
        assert_eq!(c["spec"]["resourceRef"]["name"], json!("x1"));
    }

    #[test]
    fn missing_namespace_defaults() {
        let claim = json!({"metadata":{"name":"c1"}});
        let xr = json!({"metadata":{"name":"x"}});
        let (_, x) = bind_claim_to_xr(&claim, &xr);
        assert_eq!(x["spec"]["claimRef"]["namespace"], json!("default"));
    }

    #[test]
    fn defaulting_fills_absent_field() {
        let claim = json!({"spec":{"name":"db1"}});
        let defaults = json!({"size": 10, "replicas": 1});
        let c = default_claim_from_xr_spec(&claim, &defaults);
        assert_eq!(c["spec"]["size"], json!(10));
        assert_eq!(c["spec"]["replicas"], json!(1));
        assert_eq!(c["spec"]["name"], json!("db1"));
    }

    #[test]
    fn defaulting_keeps_present_field() {
        let claim = json!({"spec":{"size": 5}});
        let defaults = json!({"size": 10});
        let c = default_claim_from_xr_spec(&claim, &defaults);
        assert_eq!(c["spec"]["size"], json!(5));
    }

    #[test]
    fn ns_scoped_detection() {
        assert!(is_namespace_scoped(
            &json!({"metadata":{"namespace":"x","name":"y"}})
        ));
        assert!(!is_namespace_scoped(&json!({"metadata":{"name":"y"}})));
    }

    #[test]
    fn defaulting_no_spec_creates() {
        let claim = json!({"metadata":{"name":"c"}});
        let defaults = json!({"x": 1});
        let c = default_claim_from_xr_spec(&claim, &defaults);
        assert_eq!(c["spec"]["x"], json!(1));
    }
}

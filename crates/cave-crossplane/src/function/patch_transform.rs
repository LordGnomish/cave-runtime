// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! function-patch-and-transform — invokes the patch_transform engine over
//! a RunFunctionRequest. Reads `input` as `PatchTransformInput`, applies
//! patches to each resource, and returns the rendered set under `desired.resources`.
//!
//! Upstream: function-patch-and-transform/fn.go

use crate::composition::patch_transform::{PatchTransformEngine, PatchTransformInput};
use crate::error::CrossplaneResult;
use crate::function::grpc_codec::RunFunctionRequest;
use serde_json::{json, Value};

pub fn run_patch_transform_fn(req: &RunFunctionRequest) -> CrossplaneResult<Value> {
    let input: PatchTransformInput = serde_json::from_value(req.input.clone()).unwrap_or_default();
    let composite_spec = req
        .observed
        .get("composite")
        .and_then(|v| v.get("spec"))
        .cloned()
        .unwrap_or(Value::Object(serde_json::Map::new()));
    let engine = PatchTransformEngine::new();
    let rendered = engine.render(&input, &composite_spec)?;
    let names = PatchTransformEngine::resource_names(&input);
    let mut named = serde_json::Map::new();
    for (i, r) in rendered.into_iter().enumerate() {
        let name = names
            .get(i)
            .cloned()
            .unwrap_or_else(|| format!("resource-{}", i));
        named.insert(name, json!({"resource": r}));
    }
    Ok(json!({"resources": named}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn empty_input_empty_resources() {
        let req = RunFunctionRequest::new("c", json!({}), json!({}));
        let out = run_patch_transform_fn(&req).unwrap();
        assert!(out["resources"].as_object().unwrap().is_empty());
    }

    #[test]
    fn one_resource_propagates() {
        let input = json!({
            "resources": [{
                "name": "r1",
                "base": {"spec":{}},
                "patches": [{
                    "patch_type": "FromCompositeFieldPath",
                    "from_field_path": "size",
                    "to_field_path": "spec.size",
                    "transforms": []
                }],
                "connection_details": [],
                "readiness_checks": []
            }],
            "patchSets": []
        });
        let observed = json!({"composite":{"spec":{"size": 7}}});
        let req = RunFunctionRequest::new("c", input, observed);
        let out = run_patch_transform_fn(&req).unwrap();
        assert_eq!(
            out["resources"]["r1"]["resource"]["spec"]["size"],
            json!(7)
        );
    }

    #[test]
    fn multiple_resources_named() {
        let input = json!({
            "resources": [
                {"name":"a","base":{},"patches":[],"connection_details":[],"readiness_checks":[]},
                {"name":"b","base":{},"patches":[],"connection_details":[],"readiness_checks":[]}
            ],
            "patchSets": []
        });
        let req = RunFunctionRequest::new("c", input, json!({}));
        let out = run_patch_transform_fn(&req).unwrap();
        assert!(out["resources"]["a"].is_object());
        assert!(out["resources"]["b"].is_object());
    }

    #[test]
    fn invalid_input_returns_empty() {
        let req = RunFunctionRequest::new("c", json!({"resources": "not-an-array"}), json!({}));
        let out = run_patch_transform_fn(&req).unwrap();
        assert!(out["resources"].as_object().unwrap().is_empty());
    }

    #[test]
    fn no_composite_spec_no_propagation() {
        let input = json!({
            "resources": [{
                "name":"r1",
                "base":{"spec":{}},
                "patches":[{
                    "patch_type":"FromCompositeFieldPath",
                    "from_field_path":"x",
                    "to_field_path":"spec.x",
                    "transforms":[]
                }],
                "connection_details":[],
                "readiness_checks":[]
            }],
            "patchSets": []
        });
        let req = RunFunctionRequest::new("c", input, json!({}));
        let out = run_patch_transform_fn(&req).unwrap();
        // base unchanged
        assert_eq!(out["resources"]["r1"]["resource"]["spec"], json!({}));
    }
}

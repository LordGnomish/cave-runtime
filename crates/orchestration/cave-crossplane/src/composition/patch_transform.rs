// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Built-in `function-patch-and-transform` engine — invokes the patch/transform
//! pipeline from `engine.rs` over a list of `Resources[]`.
//!
//! Upstream: function-patch-and-transform/input/v1beta1/resources.go +
//!           function-patch-and-transform/fn.go

use crate::engine::CompositionEngine;
use crate::error::CrossplaneResult;
use crate::models::{ComposedResource, PatchSet};
use serde::{Deserialize, Serialize};

/// Input to the function-patch-and-transform function — list of resources
/// plus an optional list of patch sets that resources can reference.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PatchTransformInput {
    #[serde(default)]
    pub resources: Vec<ComposedResource>,
    #[serde(default, rename = "patchSets")]
    pub patch_sets: Vec<PatchSet>,
}

pub struct PatchTransformEngine {
    pub engine: CompositionEngine,
}

impl PatchTransformEngine {
    pub fn new() -> Self {
        Self {
            engine: CompositionEngine::new(),
        }
    }

    /// Render desired composed resources from input + composite spec.
    pub fn render(
        &self,
        input: &PatchTransformInput,
        composite_spec: &serde_json::Value,
    ) -> CrossplaneResult<Vec<serde_json::Value>> {
        let mut out = Vec::new();
        for res in &input.resources {
            let mut base = res.base.clone();
            for patch in &res.patches {
                self.engine
                    .apply_patch(&mut base, patch, composite_spec, &input.patch_sets)?;
            }
            out.push(base);
        }
        Ok(out)
    }

    /// Names of resources in render order.
    pub fn resource_names(input: &PatchTransformInput) -> Vec<String> {
        input.resources.iter().map(|r| r.name.clone()).collect()
    }
}

impl Default for PatchTransformEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Patch, PatchType};
    use serde_json::json;

    fn res(name: &str, base: serde_json::Value, patches: Vec<Patch>) -> ComposedResource {
        ComposedResource {
            name: name.into(),
            base,
            patches,
            connection_details: vec![],
            readiness_checks: vec![],
        }
    }

    fn simple_patch(from: &str, to: &str) -> Patch {
        Patch {
            patch_type: PatchType::FromCompositeFieldPath,
            from_field_path: Some(from.into()),
            to_field_path: Some(to.into()),
            transforms: vec![],
            patch_set_name: None,
            combine: None,
        }
    }

    #[test]
    fn render_empty() {
        let e = PatchTransformEngine::new();
        let out = e
            .render(&PatchTransformInput::default(), &json!({}))
            .unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn render_propagates_field() {
        let e = PatchTransformEngine::new();
        let input = PatchTransformInput {
            resources: vec![res(
                "r1",
                json!({"spec":{}}),
                vec![simple_patch("size", "spec.size")],
            )],
            patch_sets: vec![],
        };
        let out = e.render(&input, &json!({"size": 10})).unwrap();
        assert_eq!(out[0]["spec"]["size"], json!(10));
    }

    #[test]
    fn render_multiple_resources() {
        let e = PatchTransformEngine::new();
        let input = PatchTransformInput {
            resources: vec![
                res("a", json!({}), vec![]),
                res("b", json!({}), vec![]),
                res("c", json!({}), vec![]),
            ],
            patch_sets: vec![],
        };
        let out = e.render(&input, &json!({})).unwrap();
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn resource_names_order() {
        let input = PatchTransformInput {
            resources: vec![res("a", json!({}), vec![]), res("b", json!({}), vec![])],
            patch_sets: vec![],
        };
        assert_eq!(
            PatchTransformEngine::resource_names(&input),
            vec!["a".to_string(), "b".to_string()]
        );
    }

    #[test]
    fn input_deserialize_camel_case() {
        let s = r#"{"resources":[],"patchSets":[{"name":"ps","patches":[]}]}"#;
        let i: PatchTransformInput = serde_json::from_str(s).unwrap();
        assert_eq!(i.patch_sets.len(), 1);
    }

    #[test]
    fn input_default_empty() {
        let i: PatchTransformInput = serde_json::from_str("{}").unwrap();
        assert!(i.resources.is_empty());
        assert!(i.patch_sets.is_empty());
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Crossplane 1.x compatibility — `Composition.spec.resources[]` + patch sets,
//! preserved as `LegacyComposer` for v2 backwards-compat operation.
//!
//! Upstream: internal/controller/apiextensions/composite/composition_resources.go

use crate::engine::CompositionEngine;
use crate::error::CrossplaneResult;
use crate::models::{Composition, CompositionMode};

pub struct LegacyComposer {
    pub engine: CompositionEngine,
}

impl LegacyComposer {
    pub fn new() -> Self {
        Self {
            engine: CompositionEngine::new(),
        }
    }

    /// Returns true iff this composition is in legacy "Resources" mode.
    pub fn is_legacy(c: &Composition) -> bool {
        matches!(c.mode, CompositionMode::Resources)
    }

    /// Render in legacy mode — walks the inline `resources[]` and patch sets.
    pub fn render(
        &self,
        composition: &Composition,
        composite_spec: &serde_json::Value,
    ) -> CrossplaneResult<Vec<serde_json::Value>> {
        if !Self::is_legacy(composition) {
            // Empty render for non-legacy compositions (pipeline-mode handles itself).
            return Ok(vec![]);
        }
        self.engine.render(composition, composite_spec)
    }
}

impl Default for LegacyComposer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        ComposedResource, Composition, CompositionMode, CompositionStatus, Patch, PatchType,
        TypeRef,
    };
    use chrono::Utc;
    use serde_json::json;
    use uuid::Uuid;

    fn legacy_composition(name: &str) -> Composition {
        Composition {
            id: Uuid::new_v4(),
            name: name.into(),
            composite_type_ref: TypeRef {
                api_version: "ex.cave.io/v1".into(),
                kind: "XDb".into(),
            },
            resources: vec![ComposedResource {
                name: "r1".into(),
                base: json!({"spec":{}}),
                patches: vec![Patch {
                    patch_type: PatchType::FromCompositeFieldPath,
                    from_field_path: Some("size".into()),
                    to_field_path: Some("spec.size".into()),
                    transforms: vec![],
                    patch_set_name: None,
                    combine: None,
                }],
                connection_details: vec![],
                readiness_checks: vec![],
            }],
            pipeline: vec![],
            mode: CompositionMode::Resources,
            patch_sets: vec![],
            status: CompositionStatus::Available,
            revision: 1,
            created_at: Utc::now(),
        }
    }

    fn pipeline_composition() -> Composition {
        let mut c = legacy_composition("p");
        c.mode = CompositionMode::Pipeline;
        c
    }

    #[test]
    fn detects_legacy_mode() {
        assert!(LegacyComposer::is_legacy(&legacy_composition("x")));
    }

    #[test]
    fn detects_pipeline_mode_not_legacy() {
        assert!(!LegacyComposer::is_legacy(&pipeline_composition()));
    }

    #[test]
    fn pipeline_render_returns_empty() {
        let c = LegacyComposer::new();
        let out = c.render(&pipeline_composition(), &json!({"size": 5})).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn legacy_render_propagates_field() {
        let c = LegacyComposer::new();
        let out = c
            .render(&legacy_composition("x"), &json!({"size": 5}))
            .unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["spec"]["size"], json!(5));
    }

    #[test]
    fn legacy_render_no_match_unchanged() {
        let c = LegacyComposer::new();
        let out = c.render(&legacy_composition("x"), &json!({})).unwrap();
        assert_eq!(out[0]["spec"], json!({}));
    }
}

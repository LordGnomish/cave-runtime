// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! EnvironmentConfig + in-memory composition environment.
//!
//! Line-port of crossplane v2.3.1
//! (source_sha 41c6f9c4729175cf0f953cbf267378b8734e8d27):
//!
//!   * `apis/apiextensions/v1alpha1/environmentconfig_types.go`
//!       EnvironmentConfig is a cluster-scoped key→value bag (`spec.data`).
//!   * `internal/controller/apiextensions/composite/environment/environment.go`
//!       `buildEnvironment`: the selected EnvironmentConfigs are merged, in
//!       selection order, into a single in-memory document exposed to patches
//!       under the synthetic `data.*` field path. Later configs override
//!       earlier ones key-by-key (the "merge" strategy upstream's
//!       `mergeEnvironment` implements with `mergo.Merge(..., WithOverride)`).
//!   * `internal/controller/apiextensions/composite/patches.go`
//!       `PatchTypeFromEnvironmentFieldPath` reads `from` out of the
//!       environment and writes it to the composed resource's `to` path;
//!       `PatchTypeToEnvironmentFieldPath` reads `from` out of the
//!       composite/composed object and writes it back into the environment.
//!       A missing optional source field is a no-op (not an error).
//!
//! This is a pure in-memory algorithm — no apiserver, persistence, or network
//! dependency — so it lives honestly in-crate rather than being routed to
//! cave-apiserver. (Previously skipped as `environment-configs`; converted to a
//! real tested mapped subsystem via strict TDD 2026-05-30.)

use crate::engine::{get_field_path, set_field_path};
use crate::error::CrossplaneResult;
use crate::models::{Patch, PatchType};
use std::collections::BTreeMap;

/// Cluster-scoped EnvironmentConfig: a flat key→value data bag.
///
/// Mirrors `EnvironmentConfig.Spec.Data` (a `map[string]extv1.JSON`) from
/// `environmentconfig_types.go`. We keep insertion-stable ordering via a
/// `BTreeMap` so merges are deterministic.
#[derive(Debug, Clone, PartialEq)]
pub struct EnvironmentConfig {
    pub name: String,
    pub data: BTreeMap<String, serde_json::Value>,
}

impl EnvironmentConfig {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            data: BTreeMap::new(),
        }
    }

    /// Set a key in the data bag (chainable insertion helper).
    pub fn set(&mut self, key: impl Into<String>, value: serde_json::Value) -> &mut Self {
        self.data.insert(key.into(), value);
        self
    }

    /// Read a key from the data bag.
    pub fn get(&self, key: &str) -> Option<&serde_json::Value> {
        self.data.get(key)
    }
}

/// The in-memory composition environment built from selected EnvironmentConfigs.
///
/// Upstream stores the merged environment as an unstructured object and exposes
/// it to patches under field paths; here we hold the merged data under a top
/// level `data` object so the field path `data.<key>` resolves through the same
/// dot-path walker the rest of the patch engine uses.
#[derive(Debug, Clone, Default)]
pub struct Environment {
    /// Synthetic document — `{"data": { ...merged keys... }}`.
    doc: serde_json::Value,
}

impl Environment {
    /// `buildEnvironment` — merge the selected EnvironmentConfigs in order.
    ///
    /// Port of `environment.go::buildEnvironment` + `mergeEnvironment`: each
    /// config's data keys are written into the accumulating environment with
    /// override semantics, so the **last** selected config wins on key clashes
    /// while keys only present in earlier configs survive.
    pub fn build(configs: &[EnvironmentConfig]) -> Self {
        let mut data = serde_json::Map::new();
        for cfg in configs {
            for (k, v) in &cfg.data {
                // WithOverride: later config replaces earlier value for the key.
                data.insert(k.clone(), v.clone());
            }
        }
        Self {
            doc: serde_json::Value::Object(
                [("data".to_string(), serde_json::Value::Object(data))]
                    .into_iter()
                    .collect(),
            ),
        }
    }

    /// Resolve a dot-separated field path against the environment document.
    ///
    /// Field paths are rooted at the synthetic environment, e.g. `data.region`.
    pub fn get_field_path(&self, path: &str) -> Option<serde_json::Value> {
        get_field_path(&self.doc, path)
    }

    /// `PatchTypeFromEnvironmentFieldPath` — env → composed resource.
    ///
    /// Reads `patch.from_field_path` out of the environment and writes the value
    /// to `patch.to_field_path` on `target`. Returns `Ok(true)` if a value was
    /// found and written, `Ok(false)` if the source is absent (optional patch
    /// no-op — matches upstream `IsOptionalFieldPathNotFound`).
    pub fn apply_patch(
        &self,
        target: &mut serde_json::Value,
        patch: &Patch,
    ) -> CrossplaneResult<bool> {
        debug_assert!(matches!(
            patch.patch_type,
            PatchType::FromEnvironmentFieldPath
        ));
        let (from, to) = match (&patch.from_field_path, &patch.to_field_path) {
            (Some(f), Some(t)) => (f, t),
            _ => return Ok(false),
        };
        match self.get_field_path(from) {
            Some(value) => {
                let transformed =
                    crate::engine::CompositionEngine::apply_transforms(value, &patch.transforms)?;
                set_field_path(target, to, transformed);
                Ok(true)
            }
            None => Ok(false),
        }
    }

    /// `PatchTypeToEnvironmentFieldPath` — composite/composed → env.
    ///
    /// Reads `patch.from_field_path` out of `source` and writes it back into the
    /// environment at `patch.to_field_path`. Returns `Ok(true)` on write,
    /// `Ok(false)` when the source field is absent (optional no-op).
    pub fn apply_to_environment(
        &mut self,
        source: &serde_json::Value,
        patch: &Patch,
    ) -> CrossplaneResult<bool> {
        debug_assert!(matches!(patch.patch_type, PatchType::ToEnvironmentFieldPath));
        let (from, to) = match (&patch.from_field_path, &patch.to_field_path) {
            (Some(f), Some(t)) => (f, t),
            _ => return Ok(false),
        };
        match get_field_path(source, from) {
            Some(value) => {
                let transformed =
                    crate::engine::CompositionEngine::apply_transforms(value, &patch.transforms)?;
                if !self.doc.is_object() {
                    self.doc = serde_json::Value::Object(serde_json::Map::new());
                }
                set_field_path(&mut self.doc, to, transformed);
                Ok(true)
            }
            None => Ok(false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_override_order() {
        let mut a = EnvironmentConfig::new("a");
        a.set("k", serde_json::json!(1));
        let mut b = EnvironmentConfig::new("b");
        b.set("k", serde_json::json!(2));
        let env = Environment::build(&[a, b]);
        assert_eq!(env.get_field_path("data.k"), Some(serde_json::json!(2)));
    }
}

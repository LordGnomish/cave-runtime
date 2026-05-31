// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Layered configuration merge primitive.
//!
//! Many cave-* modules need to assemble effective configuration from several
//! sources — compiled-in defaults, an on-disk config file, environment
//! variables, and CLI flags — where later sources override earlier ones. This
//! module provides a single, generic primitive for that:
//!
//! - A [`Layer`] is a named `serde_json` object tagged with its [`Source`].
//! - [`LayeredConfig::merge`] deep-merges layers in precedence order
//!   ([`Source::Default`] < [`Source::File`] < [`Source::Env`] < [`Source::Flag`]).
//! - The merge tracks **provenance**: which layer last set each top-level key.
//! - [`Merged::get`] resolves a dotted path (`"server.port"`) against the
//!   merged result.
//!
//! # Merge semantics
//! - **Objects** are merged recursively: keys present in a higher-precedence
//!   layer override matching keys in a lower one; keys unique to either side
//!   are preserved.
//! - **Scalars and arrays** are *replaced* wholesale, never merged. An array in
//!   a higher layer fully supersedes the array (or scalar) below it — there is
//!   no element-wise concatenation.
//!
//! This mirrors the resolution order documented for [`crate::config`] but works
//! on arbitrary `serde_json::Value` shapes, so it can back any module's config
//! rather than just the runtime's `CaveConfig`.

use serde_json::{Map, Value};
use std::collections::BTreeMap;

// ── Types ─────────────────────────────────────────────────────────────────────

/// Where a configuration [`Layer`] came from.
///
/// Ordering is precedence: a `Source` that compares *greater* wins on conflict.
/// `Default < File < Env < Flag`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Source {
    /// Compiled-in defaults — lowest precedence.
    Default,
    /// Values read from a config file (YAML/TOML/JSON).
    File,
    /// Values read from environment variables (e.g. `CAVE_*`).
    Env,
    /// Values supplied on the command line — highest precedence.
    Flag,
}

impl Source {
    /// Human-readable label for the source.
    pub fn as_str(&self) -> &'static str {
        match self {
            Source::Default => "default",
            Source::File => "file",
            Source::Env => "env",
            Source::Flag => "flag",
        }
    }
}

/// A single named configuration layer: a JSON object plus the source it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    /// Human-readable name for diagnostics (e.g. `"config.yaml"`, `"CAVE_* env"`).
    pub name: String,
    /// Which source category this layer belongs to — drives precedence.
    pub source: Source,
    /// The layer's key/value object.
    pub values: Map<String, Value>,
}

impl Layer {
    /// Build a layer from a name, source, and object map.
    pub fn new(name: impl Into<String>, source: Source, values: Map<String, Value>) -> Self {
        Self {
            name: name.into(),
            source,
            values,
        }
    }

    /// Build a layer from any `serde_json::Value`.
    ///
    /// Returns `None` if `value` is not a JSON object, since a layer must be a
    /// keyed map to merge meaningfully.
    pub fn from_value(name: impl Into<String>, source: Source, value: Value) -> Option<Self> {
        match value {
            Value::Object(map) => Some(Self::new(name, source, map)),
            _ => None,
        }
    }
}

/// The result of merging a set of [`Layer`]s.
#[derive(Debug, Clone, PartialEq)]
pub struct Merged {
    /// The effective configuration object.
    values: Map<String, Value>,
    /// For each top-level key, the name of the layer that last set it.
    provenance: BTreeMap<String, String>,
}

// ── Merge primitive ─────────────────────────────────────────────────────────────

/// A stack of layers awaiting merge.
#[derive(Debug, Clone, Default)]
pub struct LayeredConfig {
    layers: Vec<Layer>,
}

impl LayeredConfig {
    /// Create an empty layered config.
    pub fn new() -> Self {
        Self { layers: Vec::new() }
    }

    /// Append a layer. Insertion order is *not* significant — precedence is
    /// derived from each layer's [`Source`] at merge time.
    pub fn with_layer(mut self, layer: Layer) -> Self {
        self.layers.push(layer);
        self
    }

    /// Append a layer in place.
    pub fn push(&mut self, layer: Layer) {
        self.layers.push(layer);
    }

    /// Number of layers currently staged.
    pub fn len(&self) -> usize {
        self.layers.len()
    }

    /// Whether any layers have been staged.
    pub fn is_empty(&self) -> bool {
        self.layers.is_empty()
    }

    /// Deep-merge all layers in precedence order and record provenance.
    ///
    /// Layers are processed lowest-precedence first
    /// (`Default → File → Env → Flag`). Within the same [`Source`], layers are
    /// applied in the order they were added (a later-added layer of equal
    /// source overrides an earlier one). Object values are merged recursively;
    /// scalars and arrays replace wholesale.
    pub fn merge(&self) -> Merged {
        // Stable sort by source precedence; equal sources keep insertion order.
        let mut ordered: Vec<&Layer> = self.layers.iter().collect();
        ordered.sort_by_key(|l| l.source);

        let mut acc: Map<String, Value> = Map::new();
        let mut provenance: BTreeMap<String, String> = BTreeMap::new();

        for layer in ordered {
            for (key, value) in &layer.values {
                merge_value(&mut acc, key, value);
                // Provenance tracks the *last* layer to touch each top-level key,
                // which — given precedence ordering — is the winning source.
                provenance.insert(key.clone(), layer.name.clone());
            }
        }

        Merged {
            values: acc,
            provenance,
        }
    }
}

/// Merge a single `(key, value)` into `acc` with deep-object semantics.
fn merge_value(acc: &mut Map<String, Value>, key: &str, incoming: &Value) {
    match (acc.get_mut(key), incoming) {
        // Both sides are objects → recurse, preserving non-conflicting keys.
        (Some(Value::Object(existing)), Value::Object(incoming_obj)) => {
            for (k, v) in incoming_obj {
                merge_value(existing, k, v);
            }
        }
        // Otherwise the incoming value replaces wholesale (scalar, array, or
        // object-over-scalar / scalar-over-object).
        _ => {
            acc.insert(key.to_string(), incoming.clone());
        }
    }
}

impl Merged {
    /// Borrow the effective configuration object.
    pub fn values(&self) -> &Map<String, Value> {
        &self.values
    }

    /// Consume into the effective configuration as a `serde_json::Value::Object`.
    pub fn into_value(self) -> Value {
        Value::Object(self.values)
    }

    /// Resolve a dotted path (`"a.b.c"`) against the merged config.
    ///
    /// Traverses nested objects segment by segment. Returns `None` if any
    /// segment is missing or if an intermediate segment is not an object.
    /// An empty path returns `None`.
    pub fn get(&self, path: &str) -> Option<&Value> {
        if path.is_empty() {
            return None;
        }
        let mut segments = path.split('.');
        let first = segments.next()?;
        let mut current = self.values.get(first)?;
        for seg in segments {
            current = current.as_object()?.get(seg)?;
        }
        Some(current)
    }

    /// The name of the layer that set a given top-level `key`, if any.
    pub fn provenance(&self, key: &str) -> Option<&str> {
        self.provenance.get(key).map(|s| s.as_str())
    }

    /// Full provenance map: top-level key → winning layer name.
    pub fn provenance_map(&self) -> &BTreeMap<String, String> {
        &self.provenance
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Helper: build a layer from a JSON object literal.
    fn layer(name: &str, source: Source, value: Value) -> Layer {
        Layer::from_value(name, source, value).expect("test layer must be a JSON object")
    }

    #[test]
    fn test_higher_precedence_source_overrides_scalar() {
        let cfg = LayeredConfig::new()
            .with_layer(layer("defaults", Source::Default, json!({ "port": 8080 })))
            .with_layer(layer("flags", Source::Flag, json!({ "port": 9999 })));

        let merged = cfg.merge();
        assert_eq!(merged.get("port"), Some(&json!(9999)));
    }

    #[test]
    fn test_precedence_order_is_default_file_env_flag() {
        // Every layer sets "v"; the Flag value must win regardless of add order.
        let cfg = LayeredConfig::new()
            .with_layer(layer("flag", Source::Flag, json!({ "v": "flag" })))
            .with_layer(layer("default", Source::Default, json!({ "v": "default" })))
            .with_layer(layer("env", Source::Env, json!({ "v": "env" })))
            .with_layer(layer("file", Source::File, json!({ "v": "file" })));

        let merged = cfg.merge();
        assert_eq!(merged.get("v"), Some(&json!("flag")));
    }

    #[test]
    fn test_deep_object_merge_preserves_sibling_keys() {
        let cfg = LayeredConfig::new()
            .with_layer(layer(
                "defaults",
                Source::Default,
                json!({ "server": { "host": "0.0.0.0", "port": 8080 } }),
            ))
            .with_layer(layer(
                "env",
                Source::Env,
                json!({ "server": { "port": 9090 } }),
            ));

        let merged = cfg.merge();
        // Overridden key takes the env value...
        assert_eq!(merged.get("server.port"), Some(&json!(9090)));
        // ...while the untouched sibling key survives from defaults.
        assert_eq!(merged.get("server.host"), Some(&json!("0.0.0.0")));
    }

    #[test]
    fn test_arrays_are_replaced_not_merged() {
        let cfg = LayeredConfig::new()
            .with_layer(layer(
                "defaults",
                Source::Default,
                json!({ "hosts": ["a", "b", "c"] }),
            ))
            .with_layer(layer("flags", Source::Flag, json!({ "hosts": ["x"] })));

        let merged = cfg.merge();
        // The higher layer's array fully replaces — no concatenation.
        assert_eq!(merged.get("hosts"), Some(&json!(["x"])));
    }

    #[test]
    fn test_scalar_over_object_replaces_wholesale() {
        let cfg = LayeredConfig::new()
            .with_layer(layer(
                "defaults",
                Source::Default,
                json!({ "tls": { "enabled": true } }),
            ))
            .with_layer(layer("flags", Source::Flag, json!({ "tls": false })));

        let merged = cfg.merge();
        assert_eq!(merged.get("tls"), Some(&json!(false)));
    }

    #[test]
    fn test_provenance_tracks_winning_layer() {
        let cfg = LayeredConfig::new()
            .with_layer(layer(
                "defaults",
                Source::Default,
                json!({ "port": 8080, "host": "0.0.0.0" }),
            ))
            .with_layer(layer("config.yaml", Source::File, json!({ "host": "1.2.3.4" })))
            .with_layer(layer("CAVE_* env", Source::Env, json!({ "port": 9090 })));

        let merged = cfg.merge();
        // "port" last touched by the env layer (Env > Default).
        assert_eq!(merged.provenance("port"), Some("CAVE_* env"));
        // "host" last touched by the file layer (File > Default).
        assert_eq!(merged.provenance("host"), Some("config.yaml"));
        // Unknown key has no provenance.
        assert_eq!(merged.provenance("nope"), None);
    }

    #[test]
    fn test_same_source_later_layer_wins() {
        // Two layers of equal precedence: insertion order breaks the tie.
        let cfg = LayeredConfig::new()
            .with_layer(layer("env-base", Source::Env, json!({ "k": 1 })))
            .with_layer(layer("env-override", Source::Env, json!({ "k": 2 })));

        let merged = cfg.merge();
        assert_eq!(merged.get("k"), Some(&json!(2)));
        assert_eq!(merged.provenance("k"), Some("env-override"));
    }

    #[test]
    fn test_dotted_get_traverses_nested_objects() {
        let cfg = LayeredConfig::new().with_layer(layer(
            "defaults",
            Source::Default,
            json!({ "a": { "b": { "c": 42 } } }),
        ));

        let merged = cfg.merge();
        assert_eq!(merged.get("a.b.c"), Some(&json!(42)));
        // Intermediate node is itself fetchable.
        assert_eq!(merged.get("a.b"), Some(&json!({ "c": 42 })));
    }

    #[test]
    fn test_dotted_get_missing_and_non_object_segments() {
        let cfg = LayeredConfig::new().with_layer(layer(
            "defaults",
            Source::Default,
            json!({ "a": { "b": 1 }, "scalar": 5 }),
        ));

        let merged = cfg.merge();
        // Missing leaf.
        assert_eq!(merged.get("a.missing"), None);
        // Descending into a scalar fails rather than panicking.
        assert_eq!(merged.get("scalar.deeper"), None);
        // Empty path is None.
        assert_eq!(merged.get(""), None);
    }

    #[test]
    fn test_disjoint_keys_from_multiple_layers_all_present() {
        let cfg = LayeredConfig::new()
            .with_layer(layer("defaults", Source::Default, json!({ "a": 1 })))
            .with_layer(layer("file", Source::File, json!({ "b": 2 })))
            .with_layer(layer("env", Source::Env, json!({ "c": 3 })));

        let merged = cfg.merge();
        assert_eq!(merged.get("a"), Some(&json!(1)));
        assert_eq!(merged.get("b"), Some(&json!(2)));
        assert_eq!(merged.get("c"), Some(&json!(3)));
        assert_eq!(merged.provenance_map().len(), 3);
    }

    #[test]
    fn test_from_value_rejects_non_object() {
        assert!(Layer::from_value("x", Source::Flag, json!([1, 2, 3])).is_none());
        assert!(Layer::from_value("x", Source::Flag, json!(42)).is_none());
        assert!(Layer::from_value("x", Source::Flag, json!({})).is_some());
    }

    #[test]
    fn test_source_ordering() {
        assert!(Source::Default < Source::File);
        assert!(Source::File < Source::Env);
        assert!(Source::Env < Source::Flag);
    }

    #[test]
    fn test_empty_merge_is_empty() {
        let merged = LayeredConfig::new().merge();
        assert!(merged.values().is_empty());
        assert_eq!(merged.get("anything"), None);
    }
}
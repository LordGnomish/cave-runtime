// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors

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
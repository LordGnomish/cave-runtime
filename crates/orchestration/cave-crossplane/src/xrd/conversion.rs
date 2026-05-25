// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! XRD v1 ↔ v2 conversion + version routing.
//!
//! Upstream: apis/apiextensions/v2/conversion.go
//!
//! v2 introduces `defaultCompositeDeletePolicy` + relocates `connectionSecretKeys`
//! and renames a few sub-fields. We perform a best-effort merging conversion so
//! that v1 XRDs and v2 XRDs both flow through the same `XrdStore`.

use serde_json::Value;

/// Convert a v1 XRD JSON to v2 JSON shape.
pub fn convert_v1_to_v2(v1: &Value) -> Value {
    let mut v2 = v1.clone();
    set_api_version(&mut v2, "apiextensions.crossplane.io/v2");
    // v2 defaults: defaultCompositeDeletePolicy = "Background"
    if let Some(spec) = v2
        .as_object_mut()
        .and_then(|o| o.get_mut("spec"))
        .and_then(|s| s.as_object_mut())
    {
        if !spec.contains_key("defaultCompositeDeletePolicy") {
            spec.insert(
                "defaultCompositeDeletePolicy".into(),
                Value::String("Background".into()),
            );
        }
    }
    v2
}

/// Convert a v2 XRD JSON to v1 JSON shape — drops v2-only fields.
pub fn convert_v2_to_v1(v2: &Value) -> Value {
    let mut v1 = v2.clone();
    set_api_version(&mut v1, "apiextensions.crossplane.io/v1");
    if let Some(spec) = v1
        .as_object_mut()
        .and_then(|o| o.get_mut("spec"))
        .and_then(|s| s.as_object_mut())
    {
        spec.remove("defaultCompositeDeletePolicy");
    }
    v1
}

fn set_api_version(v: &mut Value, version: &str) {
    if let Some(o) = v.as_object_mut() {
        o.insert("apiVersion".into(), Value::String(version.into()));
    }
}

/// Detect XRD api-version.
pub fn detect_version(xrd: &Value) -> XrdApiVersion {
    let s = xrd.get("apiVersion").and_then(|v| v.as_str()).unwrap_or("");
    if s.ends_with("/v2") {
        XrdApiVersion::V2
    } else if s.ends_with("/v1") {
        XrdApiVersion::V1
    } else {
        XrdApiVersion::Unknown
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XrdApiVersion {
    V1,
    V2,
    Unknown,
}

/// Pick the storage version from a list of XrdSpecVersion JSON objects.
pub fn storage_version<'a>(versions: &'a [Value]) -> Option<&'a Value> {
    versions
        .iter()
        .find(|v| v.get("referenceable").and_then(|x| x.as_bool()) == Some(true))
        .or_else(|| versions.iter().find(|v| v.get("served").and_then(|x| x.as_bool()) == Some(true)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn v1_to_v2_sets_apiversion() {
        let v1 = json!({"apiVersion":"apiextensions.crossplane.io/v1","spec":{}});
        let v2 = convert_v1_to_v2(&v1);
        assert_eq!(
            v2["apiVersion"],
            json!("apiextensions.crossplane.io/v2")
        );
    }

    #[test]
    fn v1_to_v2_inserts_default_delete_policy() {
        let v1 = json!({"apiVersion":"apiextensions.crossplane.io/v1","spec":{}});
        let v2 = convert_v1_to_v2(&v1);
        assert_eq!(v2["spec"]["defaultCompositeDeletePolicy"], json!("Background"));
    }

    #[test]
    fn v2_to_v1_drops_v2_only() {
        let v2 = json!({"apiVersion":"x/v2","spec":{"defaultCompositeDeletePolicy":"Foreground"}});
        let v1 = convert_v2_to_v1(&v2);
        assert!(v1["spec"].get("defaultCompositeDeletePolicy").is_none());
    }

    #[test]
    fn detect_v1() {
        let v1 = json!({"apiVersion":"x/v1"});
        assert_eq!(detect_version(&v1), XrdApiVersion::V1);
    }

    #[test]
    fn detect_v2() {
        let v2 = json!({"apiVersion":"x/v2"});
        assert_eq!(detect_version(&v2), XrdApiVersion::V2);
    }

    #[test]
    fn detect_unknown() {
        let u = json!({"apiVersion":"x/v3"});
        assert_eq!(detect_version(&u), XrdApiVersion::Unknown);
    }

    #[test]
    fn storage_version_prefers_referenceable() {
        let vs = vec![
            json!({"name":"v1","referenceable":true,"served":true}),
            json!({"name":"v2","referenceable":false,"served":true}),
        ];
        assert_eq!(
            storage_version(&vs).unwrap()["name"],
            json!("v1")
        );
    }

    #[test]
    fn storage_version_falls_back_to_served() {
        let vs = vec![
            json!({"name":"v1","referenceable":false,"served":true}),
            json!({"name":"v2","referenceable":false,"served":false}),
        ];
        assert_eq!(storage_version(&vs).unwrap()["name"], json!("v1"));
    }

    #[test]
    fn preserves_already_set_policy() {
        let v1 = json!({"apiVersion":"x/v1","spec":{"defaultCompositeDeletePolicy":"Foreground"}});
        let v2 = convert_v1_to_v2(&v1);
        assert_eq!(v2["spec"]["defaultCompositeDeletePolicy"], json!("Foreground"));
    }
}

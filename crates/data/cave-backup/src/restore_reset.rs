// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Restore object metadata/status reset transforms.
//!
//! Faithful line-port of three pure pieces of Velero's restore path
//! (`pkg/restore/restore.go`, Apache-2.0):
//!   * `resetMetadata`          — strip server-populated identity fields.
//!   * `resetStatus`            — drop the top-level `status` subresource.
//!   * `resetMetadataAndStatus` — convenience wrapper running both.
//!
//! Before a backed-up Kubernetes object is re-created on restore, Velero clears
//! the fields the API server owns (uid, resourceVersion, ownerReferences, …) and
//! removes any `status`, so the object is admitted as if freshly created. This
//! is pure in-memory transformation over an unstructured object (modelled here
//! as a [`serde_json::Value`]); no discovery, plugin RPC, or persistence.

use serde_json::Value;

/// Metadata keys removed by [`reset_metadata`]. Port of the `switch` in Velero
/// `resetMetadata`: a blacklist — these server/identity fields are deleted and
/// every other metadata key (name, namespace, labels, annotations, managedFields,
/// finalizers, …) is preserved.
const STRIPPED_METADATA_KEYS: &[&str] = &[
    "generateName",
    "selfLink",
    "uid",
    "resourceVersion",
    "generation",
    "creationTimestamp",
    "deletionTimestamp",
    "deletionGracePeriodSeconds",
    "ownerReferences",
];

/// JSON type name for the `metadata was of type %T` error, mirroring the shape of
/// Velero's `%T` formatting (Go would print the concrete type).
fn json_type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Remove the server-populated identity fields from `obj["metadata"]`, mutating
/// `obj` in place. Port of Velero `resetMetadata`.
///
/// Errors (verbatim upstream strings):
///   * `"metadata not found"` when there is no `metadata` key.
///   * `"metadata was of type {ty}, expected map[string]any"` when `metadata`
///     is present but is not an object.
pub fn reset_metadata(obj: &mut Value) -> Result<(), String> {
    let res = match obj.get("metadata") {
        Some(v) => v,
        None => return Err("metadata not found".to_string()),
    };
    if !res.is_object() {
        return Err(format!(
            "metadata was of type {}, expected map[string]any",
            json_type_name(res)
        ));
    }
    // Safe: we just confirmed it is an object.
    let metadata = obj
        .get_mut("metadata")
        .and_then(Value::as_object_mut)
        .expect("metadata confirmed object");
    for k in STRIPPED_METADATA_KEYS {
        metadata.remove(*k);
    }
    Ok(())
}

/// Remove the top-level `status` subresource from `obj`, mutating in place.
/// No-op when absent. Port of Velero `resetStatus`.
pub fn reset_status(obj: &mut Value) {
    if let Some(map) = obj.as_object_mut() {
        map.remove("status");
    }
}

/// Run [`reset_metadata`] then [`reset_status`]. Port of Velero
/// `resetMetadataAndStatus`: if metadata reset errors, the error is returned and
/// `status` is left untouched.
pub fn reset_metadata_and_status(obj: &mut Value) -> Result<(), String> {
    reset_metadata(obj)?;
    reset_status(obj);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn strips_only_blacklisted_keys() {
        let mut obj = json!({"metadata": {"name": "n", "labels": {"a": "b"}, "uid": "x"}});
        reset_metadata(&mut obj).unwrap();
        let md = obj["metadata"].as_object().unwrap();
        assert!(md.contains_key("name"));
        assert!(md.contains_key("labels"));
        assert!(!md.contains_key("uid"));
    }

    #[test]
    fn wrong_type_reports_json_type() {
        let mut obj = json!({"metadata": ["x"]});
        let err = reset_metadata(&mut obj).unwrap_err();
        assert_eq!(err, "metadata was of type array, expected map[string]any");
    }
}

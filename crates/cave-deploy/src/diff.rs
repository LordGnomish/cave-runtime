//! Diff engine — live vs desired state, structured merge diff.

use serde_json::Value;
use std::collections::HashMap;

/// A single diff entry between desired and live state.
#[derive(Debug, Clone, PartialEq)]
pub struct DiffEntry {
    pub path: String,
    pub diff_type: DiffType,
    pub desired: Option<Value>,
    pub live: Option<Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DiffType {
    Added,
    Removed,
    Modified,
}

/// Result of a full resource diff.
#[derive(Debug, Clone)]
pub struct DiffResult {
    pub resource_key: String,
    pub in_sync: bool,
    pub entries: Vec<DiffEntry>,
    pub normalized_desired: Value,
    pub normalized_live: Value,
}

/// Compute the diff between desired and live JSON values.
pub fn compute_diff(desired: &Value, live: &Value) -> Vec<DiffEntry> {
    let mut entries = Vec::new();
    diff_recursive(desired, live, "", &mut entries);
    entries
}

fn diff_recursive(desired: &Value, live: &Value, path: &str, entries: &mut Vec<DiffEntry>) {
    match (desired, live) {
        (Value::Object(d_map), Value::Object(l_map)) => {
            // Check all desired keys
            for (k, d_val) in d_map {
                let child_path = if path.is_empty() { k.clone() } else { format!("{}.{}", path, k) };
                match l_map.get(k) {
                    None => entries.push(DiffEntry {
                        path: child_path,
                        diff_type: DiffType::Added,
                        desired: Some(d_val.clone()),
                        live: None,
                    }),
                    Some(l_val) => diff_recursive(d_val, l_val, &child_path, entries),
                }
            }
            // Keys in live but not in desired
            for (k, l_val) in l_map {
                if !d_map.contains_key(k) {
                    let child_path = if path.is_empty() { k.clone() } else { format!("{}.{}", path, k) };
                    entries.push(DiffEntry {
                        path: child_path,
                        diff_type: DiffType::Removed,
                        desired: None,
                        live: Some(l_val.clone()),
                    });
                }
            }
        }
        (Value::Array(d_arr), Value::Array(l_arr)) => {
            let max_len = d_arr.len().max(l_arr.len());
            for i in 0..max_len {
                let child_path = format!("{}[{}]", path, i);
                match (d_arr.get(i), l_arr.get(i)) {
                    (Some(d), Some(l)) => diff_recursive(d, l, &child_path, entries),
                    (Some(d), None) => entries.push(DiffEntry {
                        path: child_path,
                        diff_type: DiffType::Added,
                        desired: Some(d.clone()),
                        live: None,
                    }),
                    (None, Some(l)) => entries.push(DiffEntry {
                        path: child_path,
                        diff_type: DiffType::Removed,
                        desired: None,
                        live: Some(l.clone()),
                    }),
                    (None, None) => {}
                }
            }
        }
        _ => {
            if desired != live {
                entries.push(DiffEntry {
                    path: path.to_string(),
                    diff_type: DiffType::Modified,
                    desired: Some(desired.clone()),
                    live: Some(live.clone()),
                });
            }
        }
    }
}

/// Normalize a Kubernetes resource for diffing by removing server-side fields.
pub fn normalize_resource(resource: &Value) -> Value {
    let mut normalized = resource.clone();
    if let Value::Object(map) = &mut normalized {
        // Remove server-managed metadata fields
        if let Some(Value::Object(meta)) = map.get_mut("metadata") {
            meta.remove("creationTimestamp");
            meta.remove("resourceVersion");
            meta.remove("uid");
            meta.remove("generation");
            meta.remove("managedFields");
            // Remove server-injected annotations
            if let Some(Value::Object(annots)) = meta.get_mut("annotations") {
                annots.remove("kubectl.kubernetes.io/last-applied-configuration");
                annots.remove("deployment.kubernetes.io/revision");
            }
        }
        // Remove server-populated status
        map.remove("status");
    }
    normalized
}

/// Apply ignored differences to filter out known acceptable diffs.
pub fn apply_ignored_differences(
    entries: Vec<DiffEntry>,
    ignored: &[IgnoredDiff],
) -> Vec<DiffEntry> {
    entries.into_iter().filter(|e| {
        !ignored.iter().any(|ig| ig.matches(&e.path))
    }).collect()
}

#[derive(Debug, Clone)]
pub struct IgnoredDiff {
    pub json_pointer: Option<String>,
    pub jq_expression: Option<String>,
}

impl IgnoredDiff {
    pub fn matches(&self, path: &str) -> bool {
        if let Some(ref ptr) = self.json_pointer {
            // Simple prefix match for JSON pointer
            let normalized_ptr = ptr.trim_start_matches('/').replace('/', ".");
            return path.starts_with(&normalized_ptr) || &normalized_ptr == path;
        }
        false
    }
}

/// Summary of diff state for an application.
#[derive(Debug, Clone)]
pub struct ApplicationDiff {
    pub application_name: String,
    pub total_resources: usize,
    pub out_of_sync: usize,
    pub missing: usize,
    pub extra: usize,
    pub resource_diffs: Vec<ResourceDiffSummary>,
}

#[derive(Debug, Clone)]
pub struct ResourceDiffSummary {
    pub kind: String,
    pub namespace: String,
    pub name: String,
    pub in_sync: bool,
    pub diff_count: usize,
}

impl ApplicationDiff {
    pub fn is_in_sync(&self) -> bool {
        self.out_of_sync == 0 && self.missing == 0 && self.extra == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn diff_identical_values() {
        let a = json!({"key": "value"});
        let diffs = compute_diff(&a, &a);
        assert!(diffs.is_empty());
    }

    #[test]
    fn diff_added_key() {
        let desired = json!({"key": "value", "new": "field"});
        let live = json!({"key": "value"});
        let diffs = compute_diff(&desired, &live);
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].diff_type, DiffType::Added);
        assert_eq!(diffs[0].path, "new");
    }

    #[test]
    fn diff_removed_key() {
        let desired = json!({"key": "value"});
        let live = json!({"key": "value", "extra": "field"});
        let diffs = compute_diff(&desired, &live);
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].diff_type, DiffType::Removed);
    }

    #[test]
    fn diff_modified_value() {
        let desired = json!({"replicas": 3});
        let live = json!({"replicas": 1});
        let diffs = compute_diff(&desired, &live);
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].diff_type, DiffType::Modified);
        assert_eq!(diffs[0].path, "replicas");
    }

    #[test]
    fn diff_nested_path() {
        let desired = json!({"spec": {"replicas": 3}});
        let live = json!({"spec": {"replicas": 1}});
        let diffs = compute_diff(&desired, &live);
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].path, "spec.replicas");
    }

    #[test]
    fn diff_array_elements() {
        let desired = json!({"containers": [{"image": "nginx:1.25"}]});
        let live = json!({"containers": [{"image": "nginx:1.24"}]});
        let diffs = compute_diff(&desired, &live);
        assert_eq!(diffs.len(), 1);
        assert!(diffs[0].path.contains("image"));
    }

    #[test]
    fn normalize_removes_server_fields() {
        let resource = json!({
            "apiVersion": "apps/v1",
            "kind": "Deployment",
            "metadata": {
                "name": "my-app",
                "resourceVersion": "12345",
                "uid": "abc-def",
                "creationTimestamp": "2024-01-01",
                "managedFields": [],
                "generation": 5
            },
            "spec": { "replicas": 2 },
            "status": { "availableReplicas": 2 }
        });
        let normalized = normalize_resource(&resource);
        let meta = normalized.get("metadata").unwrap();
        assert!(meta.get("resourceVersion").is_none());
        assert!(meta.get("uid").is_none());
        assert!(meta.get("managedFields").is_none());
        assert!(normalized.get("status").is_none());
        assert_eq!(normalized["spec"]["replicas"], 2);
    }

    #[test]
    fn apply_ignored_differences() {
        let entries = vec![
            DiffEntry { path: "metadata.annotations.kubectl.kubernetes.io/last-applied-configuration".to_string(), diff_type: DiffType::Modified, desired: None, live: None },
            DiffEntry { path: "spec.replicas".to_string(), diff_type: DiffType::Modified, desired: Some(json!(3)), live: Some(json!(1)) },
        ];
        let ignored = vec![
            IgnoredDiff { json_pointer: Some("/metadata/annotations".to_string()), jq_expression: None },
        ];
        let remaining = super::apply_ignored_differences(entries, &ignored);
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].path, "spec.replicas");
    }

    #[test]
    fn application_diff_in_sync() {
        let diff = ApplicationDiff {
            application_name: "my-app".to_string(),
            total_resources: 5,
            out_of_sync: 0,
            missing: 0,
            extra: 0,
            resource_diffs: vec![],
        };
        assert!(diff.is_in_sync());
    }

    #[test]
    fn application_diff_out_of_sync() {
        let diff = ApplicationDiff {
            application_name: "my-app".to_string(),
            total_resources: 5,
            out_of_sync: 2,
            missing: 0,
            extra: 0,
            resource_diffs: vec![],
        };
        assert!(!diff.is_in_sync());
    }
}

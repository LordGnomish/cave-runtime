//! Diff engine — compares desired state (from Git) to live state (from cluster).
//!
//! Produces `ResourceDiff` entries that show exactly what would change before
//! a sync is applied, equivalent to `argocd app diff`.

use crate::models::{DiffType, Manifest, ResourceDiff};
use serde_json::Value;
use std::collections::HashMap;

/// A key that uniquely identifies a Kubernetes resource.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ResourceKey {
    pub api_version: String,
    pub kind: String,
    pub namespace: Option<String>,
    pub name: String,
}

impl ResourceKey {
    pub fn from_manifest(m: &Manifest) -> Self {
        Self {
            api_version: m.api_version.clone(),
            kind: m.kind.clone(),
            namespace: m.namespace.clone(),
            name: m.name.clone(),
        }
    }

    pub fn from_live(live: &Value) -> Option<Self> {
        Some(Self {
            api_version: live["apiVersion"].as_str()?.to_string(),
            kind: live["kind"].as_str()?.to_string(),
            namespace: live["metadata"]["namespace"].as_str().map(String::from),
            name: live["metadata"]["name"].as_str()?.to_string(),
        })
    }
}

/// Compute the diff between desired manifests (from Git) and live objects
/// (from the cluster).  Returns one `ResourceDiff` per resource.
pub fn compute_diff(
    desired: &[Manifest],
    live: &[Value],
) -> Vec<ResourceDiff> {
    let mut live_map: HashMap<ResourceKey, Value> = HashMap::new();
    for obj in live {
        if let Some(key) = ResourceKey::from_live(obj) {
            live_map.insert(key, obj.clone());
        }
    }

    let mut diffs: Vec<ResourceDiff> = Vec::new();
    let mut desired_keys: std::collections::HashSet<ResourceKey> =
        std::collections::HashSet::new();

    for m in desired {
        let key = ResourceKey::from_manifest(m);
        desired_keys.insert(key.clone());

        let (group, version) = split_api_version(&m.api_version);

        match live_map.get(&key) {
            None => {
                diffs.push(ResourceDiff {
                    group: group.clone(),
                    version: version.clone(),
                    kind: m.kind.clone(),
                    namespace: m.namespace.clone(),
                    name: m.name.clone(),
                    diff_type: DiffType::Added,
                    desired: Some(normalize_desired(&m.raw)),
                    live: None,
                    patch: Some(format!("+++ {}/{} added", m.kind, m.name)),
                });
            }
            Some(live_obj) => {
                let normalized_desired = normalize_desired(&m.raw);
                let normalized_live = normalize_live(live_obj);
                if objects_differ(&normalized_desired, &normalized_live) {
                    let patch = build_patch(&normalized_live, &normalized_desired);
                    diffs.push(ResourceDiff {
                        group: group.clone(),
                        version: version.clone(),
                        kind: m.kind.clone(),
                        namespace: m.namespace.clone(),
                        name: m.name.clone(),
                        diff_type: DiffType::Modified,
                        desired: Some(normalized_desired),
                        live: Some(normalized_live),
                        patch: Some(patch),
                    });
                } else {
                    diffs.push(ResourceDiff {
                        group: group.clone(),
                        version: version.clone(),
                        kind: m.kind.clone(),
                        namespace: m.namespace.clone(),
                        name: m.name.clone(),
                        diff_type: DiffType::Unchanged,
                        desired: Some(normalized_desired),
                        live: Some(normalized_live),
                        patch: None,
                    });
                }
            }
        }
    }

    // Resources that are live but not in desired → Removed (would be pruned)
    for (key, live_obj) in &live_map {
        if !desired_keys.contains(key) {
            let (group, version) = split_api_version(&key.api_version);
            diffs.push(ResourceDiff {
                group: group.clone(),
                version: version.clone(),
                kind: key.kind.clone(),
                namespace: key.namespace.clone(),
                name: key.name.clone(),
                diff_type: DiffType::Removed,
                desired: None,
                live: Some(live_obj.clone()),
                patch: Some(format!("--- {}/{} removed", key.kind, key.name)),
            });
        }
    }

    diffs
}

/// Returns true when the app is out of sync (any resource is Added, Modified,
/// or Removed).
pub fn is_out_of_sync(diffs: &[ResourceDiff]) -> bool {
    diffs.iter().any(|d| {
        matches!(d.diff_type, DiffType::Added | DiffType::Modified | DiffType::Removed)
    })
}

/// Strip server-managed fields that should not be considered during diff:
/// `resourceVersion`, `uid`, `creationTimestamp`, `generation`,
/// `managedFields`, `status`.
fn normalize_live(obj: &Value) -> Value {
    let mut v = obj.clone();
    if let Some(meta) = v["metadata"].as_object_mut() {
        meta.remove("resourceVersion");
        meta.remove("uid");
        meta.remove("creationTimestamp");
        meta.remove("generation");
        meta.remove("managedFields");
        // Strip only the noisy kubectl last-applied annotation, not all annotations
        if let Some(ann) = meta.get_mut("annotations").and_then(|a| a.as_object_mut()) {
            ann.remove("kubectl.kubernetes.io/last-applied-configuration");
        }
    }
    if let Some(map) = v.as_object_mut() {
        map.remove("status");
    }
    v
}

/// Normalize the desired object similarly so comparison is fair.
fn normalize_desired(obj: &Value) -> Value {
    let mut v = obj.clone();
    if let Some(meta) = v["metadata"].as_object_mut() {
        meta.remove("creationTimestamp");
        meta.remove("resourceVersion");
        meta.remove("uid");
        meta.remove("generation");
        meta.remove("managedFields");
    }
    if let Some(map) = v.as_object_mut() {
        map.remove("status");
    }
    v
}

fn objects_differ(a: &Value, b: &Value) -> bool {
    a != b
}

/// Build a naive line-diff patch string between two JSON objects.
fn build_patch(old: &Value, new: &Value) -> String {
    let old_str = serde_json::to_string_pretty(old).unwrap_or_default();
    let new_str = serde_json::to_string_pretty(new).unwrap_or_default();

    let old_lines: Vec<&str> = old_str.lines().collect();
    let new_lines: Vec<&str> = new_str.lines().collect();

    let mut patch = String::new();
    // Simple unified diff: lines only in old are "−", only in new are "+".
    // For production use a proper diff library; this is intentionally minimal.
    for line in &old_lines {
        if !new_lines.contains(line) {
            patch.push('-');
            patch.push_str(line);
            patch.push('\n');
        }
    }
    for line in &new_lines {
        if !old_lines.contains(line) {
            patch.push('+');
            patch.push_str(line);
            patch.push('\n');
        }
    }
    patch
}

/// Split "apps/v1" → (Some("apps"), "v1"), "v1" → (None, "v1")
pub fn split_api_version(api_version: &str) -> (Option<String>, String) {
    if let Some((g, v)) = api_version.split_once('/') {
        (Some(g.to_string()), v.to_string())
    } else {
        (None, api_version.to_string())
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Manifest;
    use serde_json::json;

    fn make_manifest(kind: &str, name: &str, ns: Option<&str>, raw: Value) -> Manifest {
        Manifest {
            api_version: "apps/v1".to_string(),
            kind: kind.to_string(),
            name: name.to_string(),
            namespace: ns.map(String::from),
            raw,
            sync_wave: 0,
            hook_type: None,
            hook_delete_policy: None,
        }
    }

    fn live_deploy(name: &str, image: &str) -> Value {
        json!({
            "apiVersion": "apps/v1",
            "kind": "Deployment",
            "metadata": { "name": name, "namespace": "default", "resourceVersion": "99" },
            "spec": { "replicas": 1, "template": { "spec": { "containers": [{"image": image}] } } }
        })
    }

    #[test]
    fn test_diff_resource_added() {
        let desired = vec![make_manifest(
            "Deployment", "myapp", Some("default"),
            json!({
                "apiVersion": "apps/v1", "kind": "Deployment",
                "metadata": {"name": "myapp", "namespace": "default"},
                "spec": {"replicas": 1}
            }),
        )];
        let live: Vec<Value> = vec![]; // nothing in cluster
        let diffs = compute_diff(&desired, &live);
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].diff_type, DiffType::Added);
    }

    #[test]
    fn test_diff_resource_removed() {
        let desired: Vec<Manifest> = vec![]; // git has nothing
        let live = vec![live_deploy("orphan", "nginx:1")];
        let diffs = compute_diff(&desired, &live);
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].diff_type, DiffType::Removed);
    }

    #[test]
    fn test_diff_resource_modified() {
        let desired = vec![make_manifest(
            "Deployment", "myapp", Some("default"),
            json!({
                "apiVersion": "apps/v1", "kind": "Deployment",
                "metadata": {"name": "myapp", "namespace": "default"},
                "spec": {"replicas": 2}
            }),
        )];
        let live = vec![live_deploy("myapp", "nginx:1")]; // replicas not set to 2
        let diffs = compute_diff(&desired, &live);
        assert_eq!(diffs.len(), 1);
        // Could be Modified or Unchanged depending on normalization; replicas differ
        assert_eq!(diffs[0].diff_type, DiffType::Modified);
    }

    #[test]
    fn test_diff_resource_unchanged() {
        let raw = json!({
            "apiVersion": "apps/v1", "kind": "Deployment",
            "metadata": {"name": "myapp", "namespace": "default"},
            "spec": {"replicas": 1, "template": {"spec": {"containers": [{"image": "nginx:1"}]}}}
        });
        let desired = vec![make_manifest("Deployment", "myapp", Some("default"), raw)];
        // Live matches exactly (minus server-managed fields which normalize strips)
        let live = vec![live_deploy("myapp", "nginx:1")];
        let diffs = compute_diff(&desired, &live);
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].diff_type, DiffType::Unchanged);
    }

    #[test]
    fn test_is_out_of_sync_true() {
        let diffs = vec![ResourceDiff {
            group: None, version: "v1".to_string(), kind: "ConfigMap".to_string(),
            namespace: None, name: "cm".to_string(), diff_type: DiffType::Modified,
            desired: None, live: None, patch: None,
        }];
        assert!(is_out_of_sync(&diffs));
    }

    #[test]
    fn test_is_out_of_sync_false() {
        let diffs = vec![ResourceDiff {
            group: None, version: "v1".to_string(), kind: "ConfigMap".to_string(),
            namespace: None, name: "cm".to_string(), diff_type: DiffType::Unchanged,
            desired: None, live: None, patch: None,
        }];
        assert!(!is_out_of_sync(&diffs));
    }
}

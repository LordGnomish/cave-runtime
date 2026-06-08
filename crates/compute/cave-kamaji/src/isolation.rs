// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Multi-tenancy security boundaries + tenant resource isolation.
//!
//! Upstream reference (Kamaji v1.0.0):
//!   internal/utilities/utilities.go   — KamajiLabels / AddTenantPrefix / MergeMaps
//!   internal/constants/labels.go      — label keys/values
//!   api/v1alpha1/datastore_types.go   — DataStoreStatus.UsedBy
//!
//! Every object Kamaji creates for a tenant is stamped with the tenant's
//! control-plane name and namespaced under it, so resources of one tenant can
//! never be selected, mutated, or reclaimed by another. This module ports the
//! labelling/prefixing primitives that enforce that boundary, plus the
//! DataStore `usedBy` registry that decides whether a back-end may be shared.

use std::collections::BTreeMap;

/// `constants/labels.go` keys.
pub const PROJECT_LABEL_KEY: &str = "kamaji.clastix.io/project";
pub const PROJECT_LABEL_VALUE: &str = "kamaji";
pub const CONTROL_PLANE_LABEL_KEY: &str = "kamaji.clastix.io/name";
pub const COMPONENT_LABEL_KEY: &str = "kamaji.clastix.io/component";

/// `utilities.KamajiLabels` — the immutable label set stamped on every
/// per-tenant resource. `kamaji.clastix.io/name` is the ownership boundary.
pub fn kamaji_labels(tcp_name: &str, resource_name: &str) -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    m.insert(PROJECT_LABEL_KEY.to_string(), PROJECT_LABEL_VALUE.to_string());
    m.insert(CONTROL_PLANE_LABEL_KEY.to_string(), tcp_name.to_string());
    m.insert(COMPONENT_LABEL_KEY.to_string(), resource_name.to_string());
    m
}

/// `utilities.AddTenantPrefix` — prefix a resource name with its owning
/// control-plane name (`{tcpName}-{name}`), so names are unique per tenant.
pub fn add_tenant_prefix(name: &str, tcp_name: &str) -> String {
    format!("{tcp_name}-{name}")
}

/// `utilities.MergeMaps` — merge in order; later maps override earlier keys.
pub fn merge_maps(maps: &[BTreeMap<String, String>]) -> BTreeMap<String, String> {
    let mut result = BTreeMap::new();
    for m in maps {
        for (k, v) in m {
            result.insert(k.clone(), v.clone());
        }
    }
    result
}

/// The label selector that scopes a list/watch to a single tenant's resources.
pub fn tenant_selector(tcp_name: &str) -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    m.insert(CONTROL_PLANE_LABEL_KEY.to_string(), tcp_name.to_string());
    m
}

/// Ownership boundary check: a resource belongs to `tcp_name` only when its
/// `kamaji.clastix.io/name` label matches. Used to refuse cross-tenant access.
pub fn owns_resource(labels: &BTreeMap<String, String>, tcp_name: &str) -> bool {
    labels
        .get(CONTROL_PLANE_LABEL_KEY)
        .map(|v| v == tcp_name)
        .unwrap_or(false)
}

/// Does a resource's label set satisfy a selector? This ports k8s
/// `labels.SelectorFromSet(selector).Matches(labels.Set(resource))`: a
/// selector built from an equality set matches iff the resource carries *every*
/// `(key, value)` pair in the selector (a superset match). An empty selector
/// matches everything. This is what scopes a tenant list/watch — the reconciler
/// lists with `client.MatchingLabels(tenant_selector(tcp))` and only the owning
/// tenant's resources come back.
pub fn selector_matches(
    resource: &BTreeMap<String, String>,
    selector: &BTreeMap<String, String>,
) -> bool {
    selector
        .iter()
        .all(|(k, v)| resource.get(k).map(|rv| rv == v).unwrap_or(false))
}

/// The `namespace/name` key a TenantControlPlane occupies in a DataStore's
/// `status.usedBy` list.
pub fn used_by_key(namespace: &str, name: &str) -> String {
    format!("{namespace}/{name}")
}

/// Record a tenant as a user of a DataStore. Returns `true` if the set changed.
/// The list is kept as a sorted set (matching the `sets.New` semantics
/// upstream uses around `status.usedBy`).
pub fn register_usage(used_by: &mut Vec<String>, key: &str) -> bool {
    if used_by.iter().any(|k| k == key) {
        return false;
    }
    used_by.push(key.to_string());
    used_by.sort();
    true
}

/// Release a tenant's claim on a DataStore. Returns `true` if it was present.
pub fn deregister_usage(used_by: &mut Vec<String>, key: &str) -> bool {
    let before = used_by.len();
    used_by.retain(|k| k != key);
    used_by.len() != before
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn selector_subset_match_scopes_to_owning_tenant() {
        // A resource owned by tenant "alpha" carries the full label set.
        let resource = kamaji_labels("alpha", "kube-apiserver");
        // The tenant list/watch selector is just the ownership key.
        let alpha = tenant_selector("alpha");
        let beta = tenant_selector("beta");
        assert!(selector_matches(&resource, &alpha));
        // Cross-tenant: beta's selector must NOT pull in alpha's resource.
        assert!(!selector_matches(&resource, &beta));
    }

    #[test]
    fn empty_selector_matches_everything() {
        let resource = kamaji_labels("alpha", "etcd");
        assert!(selector_matches(&resource, &BTreeMap::new()));
    }

    #[test]
    fn selector_requires_all_pairs_present() {
        let resource = set(&[("a", "1"), ("b", "2")]);
        assert!(selector_matches(&resource, &set(&[("a", "1")])));
        assert!(selector_matches(&resource, &set(&[("a", "1"), ("b", "2")])));
        // A key present with a different value does not match.
        assert!(!selector_matches(&resource, &set(&[("a", "9")])));
        // A selector key absent from the resource does not match.
        assert!(!selector_matches(&resource, &set(&[("c", "3")])));
    }

    #[test]
    fn selector_matches_agrees_with_owns_resource() {
        let resource = kamaji_labels("alpha", "controller-manager");
        // owns_resource is the single-key special case of selector_matches.
        assert_eq!(
            owns_resource(&resource, "alpha"),
            selector_matches(&resource, &tenant_selector("alpha"))
        );
        assert_eq!(
            owns_resource(&resource, "beta"),
            selector_matches(&resource, &tenant_selector("beta"))
        );
    }
}

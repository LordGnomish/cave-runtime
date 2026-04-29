//! ConfigMap watcher — reacts to changes in `cilium-config` and the
//! `CiliumNodeConfig` CRD.
//!
//! Mirrors `pkg/option/resolver/resolver.go`. The agent watches both
//! the cluster-wide `cilium-config` ConfigMap and the per-node
//! `CiliumNodeConfig` CRD; per-node values override cluster-wide.
//! Changes emit reconfigure events; a subset of options
//! (`enable-ipsec`, `kube-proxy-replacement`) require a full agent
//! restart.

use crate::cilium::types::{Cite, TenantId};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// Options that require a full restart on change. Mirrors
/// `pkg/option/option.go::optionsRequiringRestart`.
pub fn restart_required_options() -> BTreeSet<&'static str> {
    let mut s = BTreeSet::new();
    s.insert("enable-ipsec");
    s.insert("enable-wireguard");
    s.insert("kube-proxy-replacement");
    s.insert("tunnel");
    s.insert("ipv4-native-routing-cidr");
    s.insert("ipv6-native-routing-cidr");
    s.insert("encryption-key-rotation");
    s
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChangeAction {
    Reconfigure,
    RequireRestart,
    NoOp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigChange {
    pub key: String,
    pub from: Option<String>,
    pub to: Option<String>,
    pub action: ChangeAction,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ConfigError {
    #[error("node `{0}` not registered")]
    NodeNotFound(String),
    #[error("tenant {tenant} cannot mutate config watcher owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

#[derive(Debug)]
pub struct ConfigWatcher {
    pub tenant: TenantId,
    cluster_config: BTreeMap<String, String>,
    node_overrides: HashMap<String, BTreeMap<String, String>>,
    pending_changes: Vec<ConfigChange>,
}

impl ConfigWatcher {
    pub fn new(tenant: TenantId) -> Self {
        Self {
            tenant,
            cluster_config: BTreeMap::new(),
            node_overrides: HashMap::new(),
            pending_changes: Vec::new(),
        }
    }

    /// Update the cluster-wide ConfigMap. Compares against the current
    /// state and queues per-key changes.
    pub fn update_cluster_config(&mut self, new_config: BTreeMap<String, String>) {
        let restart = restart_required_options();
        // Detect added + changed.
        for (k, v) in &new_config {
            let prev = self.cluster_config.get(k).cloned();
            if prev.as_deref() != Some(v.as_str()) {
                let action = if restart.contains(k.as_str()) {
                    ChangeAction::RequireRestart
                } else {
                    ChangeAction::Reconfigure
                };
                self.pending_changes.push(ConfigChange {
                    key: k.clone(), from: prev, to: Some(v.clone()), action,
                });
            }
        }
        // Detect removed.
        for k in self.cluster_config.keys() {
            if !new_config.contains_key(k) {
                let prev = self.cluster_config.get(k).cloned();
                let action = if restart.contains(k.as_str()) {
                    ChangeAction::RequireRestart
                } else {
                    ChangeAction::Reconfigure
                };
                self.pending_changes.push(ConfigChange {
                    key: k.clone(), from: prev, to: None, action,
                });
            }
        }
        self.cluster_config = new_config;
    }

    pub fn set_node_override(&mut self, node: impl Into<String>, key: impl Into<String>, value: String) {
        let node = node.into();
        let key = key.into();
        let restart = restart_required_options();
        let action = if restart.contains(key.as_str()) {
            ChangeAction::RequireRestart
        } else {
            ChangeAction::Reconfigure
        };
        let entry = self.node_overrides.entry(node).or_default();
        let prev = entry.insert(key.clone(), value.clone());
        self.pending_changes.push(ConfigChange {
            key, from: prev, to: Some(value), action,
        });
    }

    pub fn remove_node_override(&mut self, node: &str, key: &str) -> bool {
        if let Some(entry) = self.node_overrides.get_mut(node) {
            let prev = entry.remove(key);
            if let Some(p) = prev {
                let restart = restart_required_options();
                let action = if restart.contains(key) {
                    ChangeAction::RequireRestart
                } else {
                    ChangeAction::Reconfigure
                };
                self.pending_changes.push(ConfigChange {
                    key: key.to_string(), from: Some(p), to: None, action,
                });
                return true;
            }
        }
        false
    }

    /// Compute the effective config for a node: cluster + overrides.
    pub fn effective(&self, node: &str) -> BTreeMap<String, String> {
        let mut out = self.cluster_config.clone();
        if let Some(overrides) = self.node_overrides.get(node) {
            for (k, v) in overrides {
                out.insert(k.clone(), v.clone());
            }
        }
        out
    }

    pub fn drain_changes(&mut self) -> Vec<ConfigChange> {
        std::mem::take(&mut self.pending_changes)
    }

    pub fn key_count(&self) -> usize {
        self.cluster_config.len()
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/option/resolver/resolver.go", "Resolver");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;

    fn watcher(tenant: TenantId) -> ConfigWatcher {
        ConfigWatcher::new(tenant)
    }

    fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| ((*k).into(), (*v).into())).collect()
    }

    // ── restart_required_options ───────────────────────────────────────────

    #[test]
    fn restart_required_includes_known_options() {
        let (_c, _t) = cilium_test_ctx!("pkg/option/option.go", "RestartRequired", "tenant-cw-rr");
        let s = restart_required_options();
        assert!(s.contains("enable-ipsec"));
        assert!(s.contains("kube-proxy-replacement"));
        assert!(s.contains("tunnel"));
        assert!(s.contains("ipv4-native-routing-cidr"));
    }

    // ── update_cluster_config ──────────────────────────────────────────────

    #[test]
    fn first_update_emits_reconfigure_per_key() {
        let (_c, tenant) = cilium_test_ctx!("pkg/option/resolver/resolver.go", "Update.First", "tenant-cw-u");
        let mut w = watcher(tenant);
        w.update_cluster_config(map(&[("a", "1"), ("b", "2")]));
        let changes = w.drain_changes();
        assert_eq!(changes.len(), 2);
        assert!(changes.iter().all(|c| matches!(c.action, ChangeAction::Reconfigure)));
    }

    #[test]
    fn changing_value_emits_reconfigure() {
        let (_c, tenant) = cilium_test_ctx!("pkg/option/resolver/resolver.go", "Update.Change", "tenant-cw-uc");
        let mut w = watcher(tenant);
        w.update_cluster_config(map(&[("a", "1")]));
        let _ = w.drain_changes();
        w.update_cluster_config(map(&[("a", "2")]));
        let changes = w.drain_changes();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].from, Some("1".into()));
        assert_eq!(changes[0].to, Some("2".into()));
    }

    #[test]
    fn changing_restart_required_option_emits_require_restart() {
        let (_c, tenant) = cilium_test_ctx!("pkg/option/resolver/resolver.go", "Update.Restart", "tenant-cw-ur");
        let mut w = watcher(tenant);
        w.update_cluster_config(map(&[("enable-ipsec", "false")]));
        let _ = w.drain_changes();
        w.update_cluster_config(map(&[("enable-ipsec", "true")]));
        let changes = w.drain_changes();
        assert_eq!(changes[0].action, ChangeAction::RequireRestart);
    }

    #[test]
    fn unchanged_value_emits_no_change() {
        let (_c, tenant) = cilium_test_ctx!("pkg/option/resolver/resolver.go", "Update.Unchanged", "tenant-cw-uu");
        let mut w = watcher(tenant);
        w.update_cluster_config(map(&[("a", "1")]));
        let _ = w.drain_changes();
        w.update_cluster_config(map(&[("a", "1")]));
        let changes = w.drain_changes();
        assert!(changes.is_empty());
    }

    #[test]
    fn removed_key_emits_change_with_to_none() {
        let (_c, tenant) = cilium_test_ctx!("pkg/option/resolver/resolver.go", "Update.Removed", "tenant-cw-rmv");
        let mut w = watcher(tenant);
        w.update_cluster_config(map(&[("a", "1"), ("b", "2")]));
        let _ = w.drain_changes();
        w.update_cluster_config(map(&[("a", "1")]));
        let changes = w.drain_changes();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].key, "b");
        assert!(changes[0].to.is_none());
    }

    // ── Node overrides ─────────────────────────────────────────────────────

    #[test]
    fn set_node_override_records_change() {
        let (_c, tenant) = cilium_test_ctx!("pkg/option/resolver/resolver.go", "NodeOverride.Set", "tenant-cw-nos");
        let mut w = watcher(tenant);
        w.set_node_override("node-a", "debug", "true".into());
        let changes = w.drain_changes();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].key, "debug");
    }

    #[test]
    fn set_node_override_for_restart_option_emits_restart() {
        let (_c, tenant) = cilium_test_ctx!("pkg/option/resolver/resolver.go", "NodeOverride.Restart", "tenant-cw-norr");
        let mut w = watcher(tenant);
        w.set_node_override("node-a", "kube-proxy-replacement", "strict".into());
        let changes = w.drain_changes();
        assert_eq!(changes[0].action, ChangeAction::RequireRestart);
    }

    #[test]
    fn remove_node_override_emits_to_none_change() {
        let (_c, tenant) = cilium_test_ctx!("pkg/option/resolver/resolver.go", "NodeOverride.Remove", "tenant-cw-nor");
        let mut w = watcher(tenant);
        w.set_node_override("node-a", "debug", "true".into());
        let _ = w.drain_changes();
        assert!(w.remove_node_override("node-a", "debug"));
        let changes = w.drain_changes();
        assert_eq!(changes[0].to, None);
        assert_eq!(changes[0].from, Some("true".into()));
    }

    #[test]
    fn remove_unknown_node_override_returns_false() {
        let (_c, tenant) = cilium_test_ctx!("pkg/option/resolver/resolver.go", "NodeOverride.Remove.NotFound", "tenant-cw-nornf");
        let mut w = watcher(tenant);
        assert!(!w.remove_node_override("node-a", "debug"));
    }

    // ── effective() ────────────────────────────────────────────────────────

    #[test]
    fn effective_returns_cluster_config_when_no_overrides() {
        let (_c, tenant) = cilium_test_ctx!("pkg/option/resolver/resolver.go", "Effective.NoOverride", "tenant-cw-eno");
        let mut w = watcher(tenant);
        w.update_cluster_config(map(&[("a", "1"), ("b", "2")]));
        let eff = w.effective("node-a");
        assert_eq!(eff.get("a").map(|s| s.as_str()), Some("1"));
        assert_eq!(eff.get("b").map(|s| s.as_str()), Some("2"));
    }

    #[test]
    fn effective_overrides_take_precedence() {
        let (_c, tenant) = cilium_test_ctx!("pkg/option/resolver/resolver.go", "Effective.Override", "tenant-cw-eo");
        let mut w = watcher(tenant);
        w.update_cluster_config(map(&[("a", "1")]));
        w.set_node_override("node-a", "a", "9".into());
        let eff = w.effective("node-a");
        assert_eq!(eff.get("a").map(|s| s.as_str()), Some("9"));
    }

    #[test]
    fn effective_overrides_only_apply_to_named_node() {
        let (_c, tenant) = cilium_test_ctx!("pkg/option/resolver/resolver.go", "Effective.PerNode", "tenant-cw-epn");
        let mut w = watcher(tenant);
        w.update_cluster_config(map(&[("a", "1")]));
        w.set_node_override("node-a", "a", "9".into());
        assert_eq!(w.effective("node-b").get("a").map(|s| s.as_str()), Some("1"));
    }

    // ── Drain semantics ────────────────────────────────────────────────────

    #[test]
    fn drain_clears_pending_changes() {
        let (_c, tenant) = cilium_test_ctx!("pkg/option/resolver/resolver.go", "Drain", "tenant-cw-d");
        let mut w = watcher(tenant);
        w.update_cluster_config(map(&[("a", "1")]));
        w.drain_changes();
        assert!(w.drain_changes().is_empty());
    }

    // ── Counts ─────────────────────────────────────────────────────────────

    #[test]
    fn key_count_tracks_cluster_config() {
        let (_c, tenant) = cilium_test_ctx!("pkg/option/resolver/resolver.go", "KeyCount", "tenant-cw-kc");
        let mut w = watcher(tenant);
        w.update_cluster_config(map(&[("a", "1"), ("b", "2"), ("c", "3")]));
        assert_eq!(w.key_count(), 3);
    }

    // ── Multi-key updates ──────────────────────────────────────────────────

    #[test]
    fn second_update_only_emits_diff() {
        let (_c, tenant) = cilium_test_ctx!("pkg/option/resolver/resolver.go", "Update.Diff", "tenant-cw-ud");
        let mut w = watcher(tenant);
        w.update_cluster_config(map(&[("a", "1"), ("b", "2"), ("c", "3")]));
        let _ = w.drain_changes();
        w.update_cluster_config(map(&[("a", "1"), ("b", "9"), ("d", "4")]));
        let changes = w.drain_changes();
        // Changes: b changed, c removed, d added.
        let keys: BTreeSet<&str> = changes.iter().map(|c| c.key.as_str()).collect();
        assert!(keys.contains("b"));
        assert!(keys.contains("c"));
        assert!(keys.contains("d"));
        assert!(!keys.contains("a"));
    }

    // ── Serde ──────────────────────────────────────────────────────────────

    #[test]
    fn config_change_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/option/resolver/resolver.go", "Change.Serde", "tenant-cw-cserde");
        let c = ConfigChange {
            key: "enable-ipsec".into(),
            from: Some("false".into()),
            to: Some("true".into()),
            action: ChangeAction::RequireRestart,
        };
        let s = serde_json::to_string(&c).unwrap();
        let back: ConfigChange = serde_json::from_str(&s).unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn change_action_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/option/resolver/resolver.go", "Action.Serde", "tenant-cw-aserde");
        for a in [ChangeAction::Reconfigure, ChangeAction::RequireRestart, ChangeAction::NoOp] {
            let s = serde_json::to_string(&a).unwrap();
            let back: ChangeAction = serde_json::from_str(&s).unwrap();
            assert_eq!(back, a);
        }
    }
}

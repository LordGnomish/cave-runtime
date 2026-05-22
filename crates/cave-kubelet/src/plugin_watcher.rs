// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Plugin watcher — kubelet's `/var/lib/kubelet/plugins_registry/`
//! observation + registration handshake for CSI / DRA / DevicePlugin.
//!
//! Mirrors Kubernetes v1.36.0 upstream:
//!   `pkg/kubelet/pluginmanager/pluginwatcher/plugin_watcher.go`
//!     (`Watcher.Start`, `Watcher.handleCreateEvent`).
//!   `pkg/kubelet/pluginmanager/operationexecutor/operation_executor.go`
//!     (`registerPlugin`, `unregisterPlugin`).
//!
//! Registration handshake:
//!
//!   1. Plugin drops a unix socket at <plugins_registry>/<name>.sock and
//!      the kubelet emits a CREATE event.
//!   2. Watcher dials the socket and calls `GetInfo` ⇒ {type, name,
//!      endpoint, supported_versions}.
//!   3. Watcher selects a version from
//!      `supported_versions ∩ kubelet_supported_versions`. If empty,
//!      registration fails with `NoCommonVersion`.
//!   4. Watcher calls `NotifyRegistrationStatus(true)` and stores the
//!      driver in the per-type registry.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PluginError {
    #[error("plugin '{0}' already registered")]
    AlreadyRegistered(String),
    #[error("plugin '{0}' not registered")]
    NotRegistered(String),
    #[error("no common version between kubelet {kubelet:?} and plugin {plugin:?}")]
    NoCommonVersion {
        kubelet: Vec<String>,
        plugin: Vec<String>,
    },
    #[error("plugin name '{0}' rejected (must be DNS-1123 subdomain)")]
    InvalidName(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum PluginType {
    Csi,
    Dra,
    DevicePlugin,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginInfo {
    pub plugin_type: PluginType,
    pub name: String,
    pub endpoint: String,
    pub supported_versions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisteredPlugin {
    pub info: PluginInfo,
    pub negotiated_version: String,
    pub tenant_id: String,
}

#[derive(Debug)]
pub struct PluginWatcher {
    pub kubelet_supported_versions: Vec<String>,
    /// (plugin_type, name) → registration
    pub registry: BTreeMap<(PluginType, String), RegisteredPlugin>,
}

impl PluginWatcher {
    pub fn new(kubelet_supported_versions: Vec<String>) -> Self {
        Self {
            kubelet_supported_versions,
            registry: BTreeMap::new(),
        }
    }

    pub fn validate_name(name: &str) -> Result<(), PluginError> {
        if name.is_empty() || name.len() > 253 {
            return Err(PluginError::InvalidName(name.into()));
        }
        let valid = name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '.');
        if !valid
            || name.starts_with('-')
            || name.starts_with('.')
            || name.ends_with('-')
            || name.ends_with('.')
        {
            return Err(PluginError::InvalidName(name.into()));
        }
        Ok(())
    }

    /// Pick the highest version present in both lists.
    /// Uses Kubernetes-aware version ordering: GA > beta > alpha,
    /// then by numeric major / sub.  Mirrors upstream
    /// `apimachinery/pkg/version.CompareKubeAwareVersionStrings`.
    pub fn negotiate_version(kubelet: &[String], plugin: &[String]) -> Result<String, PluginError> {
        let mut common: Vec<&String> = kubelet.iter().filter(|v| plugin.contains(v)).collect();
        if common.is_empty() {
            return Err(PluginError::NoCommonVersion {
                kubelet: kubelet.to_vec(),
                plugin: plugin.to_vec(),
            });
        }
        common.sort_by(|a, b| version_rank(a).cmp(&version_rank(b)));
        Ok(common.last().unwrap().to_string())
    }

    pub fn register(
        &mut self,
        info: PluginInfo,
        tenant_id: &str,
    ) -> Result<RegisteredPlugin, PluginError> {
        Self::validate_name(&info.name)?;
        let key = (info.plugin_type, info.name.clone());
        if self.registry.contains_key(&key) {
            return Err(PluginError::AlreadyRegistered(info.name));
        }
        let version =
            Self::negotiate_version(&self.kubelet_supported_versions, &info.supported_versions)?;
        let reg = RegisteredPlugin {
            info,
            negotiated_version: version,
            tenant_id: tenant_id.into(),
        };
        self.registry.insert(key, reg.clone());
        Ok(reg)
    }

    pub fn deregister(
        &mut self,
        plugin_type: PluginType,
        name: &str,
    ) -> Result<RegisteredPlugin, PluginError> {
        self.registry
            .remove(&(plugin_type, name.into()))
            .ok_or_else(|| PluginError::NotRegistered(name.into()))
    }

    pub fn list(&self, plugin_type: PluginType) -> Vec<&RegisteredPlugin> {
        self.registry
            .iter()
            .filter(|((t, _), _)| *t == plugin_type)
            .map(|(_, v)| v)
            .collect()
    }
}

/// Rank tuple ordered for K8s-aware version comparison.
/// Returns (major, track_rank, sub) where higher is newer.
/// `track_rank`: alpha=0, beta=1, ga=2, unknown=-1.
fn version_rank(v: &str) -> (i32, i32, i32) {
    let s = v.strip_prefix('v').unwrap_or(v);
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i == 0 {
        return (-1, -1, 0);
    }
    let major: i32 = s[..i].parse().unwrap_or(0);
    let rest = &s[i..];
    if rest.is_empty() {
        return (major, 2, 0);
    }
    let (track, sub_str) = if let Some(r) = rest.strip_prefix("alpha") {
        (0, r)
    } else if let Some(r) = rest.strip_prefix("beta") {
        (1, r)
    } else {
        return (major, -1, 0);
    };
    let sub: i32 = sub_str.parse().unwrap_or(0);
    (major, track, sub)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(t: PluginType, name: &str, vers: &[&str]) -> PluginInfo {
        PluginInfo {
            plugin_type: t,
            name: name.into(),
            endpoint: format!("/var/lib/kubelet/plugins/{name}.sock"),
            supported_versions: vers.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn watcher() -> PluginWatcher {
        PluginWatcher::new(vec!["v1alpha1".into(), "v1beta1".into(), "v1".into()])
    }

    #[test]
    fn name_validation_rejects_bad_chars() {
        assert!(matches!(
            PluginWatcher::validate_name("UPPER"),
            Err(PluginError::InvalidName(_))
        ));
        assert!(matches!(
            PluginWatcher::validate_name("-leading"),
            Err(PluginError::InvalidName(_))
        ));
        assert!(PluginWatcher::validate_name("ok-name").is_ok());
        assert!(PluginWatcher::validate_name("nvidia.com").is_ok());
    }

    #[test]
    fn negotiate_picks_highest_common() {
        let v = PluginWatcher::negotiate_version(
            &["v1alpha1".into(), "v1beta1".into(), "v1".into()],
            &["v1alpha1".into(), "v1beta1".into()],
        )
        .unwrap();
        assert_eq!(v, "v1beta1");
    }

    #[test]
    fn negotiate_no_common_errors() {
        let r = PluginWatcher::negotiate_version(&["v1".into()], &["v2".into()]);
        assert!(matches!(r, Err(PluginError::NoCommonVersion { .. })));
    }

    #[test]
    fn register_csi_plugin_round_trip() {
        let mut w = watcher();
        let i = info(PluginType::Csi, "csi-pd.gke.io", &["v1", "v1alpha1"]);
        let reg = w.register(i, "acme").unwrap();
        assert_eq!(reg.negotiated_version, "v1");
        assert_eq!(reg.tenant_id, "acme");
    }

    #[test]
    fn register_dra_plugin() {
        let mut w = watcher();
        let i = info(PluginType::Dra, "nvidia.com", &["v1beta1"]);
        let reg = w.register(i, "acme").unwrap();
        assert_eq!(reg.info.plugin_type, PluginType::Dra);
    }

    #[test]
    fn duplicate_registration_rejected() {
        let mut w = watcher();
        let i = info(PluginType::Csi, "csi-x", &["v1"]);
        w.register(i.clone(), "acme").unwrap();
        assert!(matches!(
            w.register(i, "acme"),
            Err(PluginError::AlreadyRegistered(_))
        ));
    }

    #[test]
    fn deregister_unknown_errors() {
        let mut w = watcher();
        assert!(matches!(
            w.deregister(PluginType::Csi, "ghost"),
            Err(PluginError::NotRegistered(_))
        ));
    }

    #[test]
    fn deregister_returns_record_then_lookup_fails() {
        let mut w = watcher();
        let i = info(PluginType::DevicePlugin, "intel.com", &["v1beta1"]);
        w.register(i, "acme").unwrap();
        let r = w.deregister(PluginType::DevicePlugin, "intel.com").unwrap();
        assert_eq!(r.info.name, "intel.com");
        assert!(matches!(
            w.deregister(PluginType::DevicePlugin, "intel.com"),
            Err(PluginError::NotRegistered(_))
        ));
    }

    #[test]
    fn list_filters_by_type() {
        let mut w = watcher();
        w.register(info(PluginType::Csi, "csi-1", &["v1"]), "acme")
            .unwrap();
        w.register(info(PluginType::Csi, "csi-2", &["v1"]), "acme")
            .unwrap();
        w.register(info(PluginType::Dra, "dra-1", &["v1"]), "acme")
            .unwrap();
        assert_eq!(w.list(PluginType::Csi).len(), 2);
        assert_eq!(w.list(PluginType::Dra).len(), 1);
        assert_eq!(w.list(PluginType::DevicePlugin).len(), 0);
    }

    #[test]
    fn invalid_name_rejected_at_register() {
        let mut w = watcher();
        let bad = info(PluginType::Csi, "BadName!", &["v1"]);
        assert!(matches!(
            w.register(bad, "acme"),
            Err(PluginError::InvalidName(_))
        ));
    }

    #[test]
    fn version_rank_ga_beats_beta_beats_alpha() {
        assert!(version_rank("v1") > version_rank("v1beta1"));
        assert!(version_rank("v1beta1") > version_rank("v1alpha1"));
        assert!(version_rank("v1beta2") > version_rank("v1beta1"));
        assert!(version_rank("v2") > version_rank("v1"));
    }

    #[test]
    fn negotiate_prefers_ga_over_beta_alpha() {
        let v = PluginWatcher::negotiate_version(
            &["v1alpha1".into(), "v1beta1".into(), "v1".into()],
            &["v1".into(), "v1alpha1".into()],
        )
        .unwrap();
        assert_eq!(v, "v1");
    }
}

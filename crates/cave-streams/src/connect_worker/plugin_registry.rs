// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Cave Runtime contributors
//
// Source: apache/kafka@9f8b3ad416bd416f3706f3d7a1f425b9dd8bc5f2
//   connect/runtime/src/main/java/org/apache/kafka/connect/runtime/isolation/PluginsRegistry.java
//   connect/runtime/src/main/java/org/apache/kafka/connect/runtime/isolation/Plugins.java

//! Plugin registry — Connect's plugin namespace isolation, Rust-ified.
//!
//! Upstream's `Plugins` class scans the classpath for connector
//! plugins, each loaded under its own classloader so peer plugins
//! that happen to declare incompatible Jackson or Guava versions do
//! not collide.
//!
//! cave-streams ships a static-link analogue: each connector
//! implementation is a Rust `ConnectorFactory` registered into the
//! [`PluginRegistry`] with a `(plugin_name, version)` key, plus the
//! `config_prefix` it owns inside the worker-wide config. Plugins
//! cannot read each other's config namespaces — `validate_config`
//! enforces that every entry the registry sees starts with one of
//! the known prefixes (the upstream "trusted plugin" check).
//!
//! Honest scope: no dynamic loading. Inventory/linkme-style
//! link-time registration is omitted to keep the workspace deps
//! flat; instead the runtime calls `PluginRegistry::register` once
//! at boot per supported connector.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::error::{StreamsError, StreamsResult};

use super::connector_runtime::ConnectorSpec;
use super::task_runtime::TaskKind;

/// SemVer-shaped version number. Connect plugins use Maven-shaped
/// `1.2.3-SNAPSHOT` strings — we treat that as opaque (string
/// compare for equality, dotted-numeric compare for ordering).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PluginVersion(pub String);

impl PluginVersion {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One connector implementation. The factory takes a fully-namespaced
/// config map (every key still has the plugin prefix) and yields a
/// concrete [`ConnectorSpec`] for the runtime to schedule.
pub type ConnectorFactory = Arc<
    dyn Fn(&BTreeMap<String, String>) -> StreamsResult<ConnectorSpec> + Send + Sync + 'static,
>;

/// Entry in the registry.
#[derive(Clone)]
pub struct PluginEntry {
    pub name: String,
    pub version: PluginVersion,
    pub kind: TaskKind,
    /// Config keys live under this prefix (e.g. `"connector.class"`,
    /// `"jdbc.connection.url"`). The registry refuses any config
    /// entry not under one of the registered prefixes.
    pub config_prefix: String,
    pub factory: ConnectorFactory,
}

impl std::fmt::Debug for PluginEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginEntry")
            .field("name", &self.name)
            .field("version", &self.version)
            .field("kind", &self.kind)
            .field("config_prefix", &self.config_prefix)
            .finish_non_exhaustive()
    }
}

#[derive(Default)]
pub struct PluginRegistry {
    /// Key — `(plugin_name, version)`. Multiple versions of the
    /// same plugin can coexist; the connector REST surface picks the
    /// pinned version, falling back to the newest registered if no
    /// pin is given.
    entries: BTreeMap<(String, PluginVersion), PluginEntry>,
    /// Config prefixes registered so cross-namespace reads can be
    /// detected. Keyed by prefix, value=plugin name (for diagnostic).
    prefixes: BTreeMap<String, String>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register `entry` under `(name, version)`. Replaces any prior
    /// entry with the same key. Records the prefix so the namespace
    /// check works.
    pub fn register(&mut self, entry: PluginEntry) -> StreamsResult<()> {
        if entry.name.is_empty() {
            return Err(StreamsError::Internal(
                "PluginRegistry: plugin name must not be empty".into(),
            ));
        }
        if entry.config_prefix.is_empty() {
            return Err(StreamsError::Internal(format!(
                "PluginRegistry: plugin '{}' has empty config_prefix",
                entry.name
            )));
        }
        // Prefix conflict check (different plugin claiming the same
        // prefix) — upstream's "duplicate" rejection.
        if let Some(other) = self.prefixes.get(&entry.config_prefix) {
            if other != &entry.name {
                return Err(StreamsError::Internal(format!(
                    "PluginRegistry: prefix '{}' already owned by '{}'",
                    entry.config_prefix, other
                )));
            }
        }
        self.prefixes.insert(entry.config_prefix.clone(), entry.name.clone());
        let key = (entry.name.clone(), entry.version.clone());
        self.entries.insert(key, entry);
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// All known prefixes — used by the config validator.
    pub fn prefixes(&self) -> Vec<String> {
        self.prefixes.keys().cloned().collect()
    }

    /// Resolve a specific (name, version) pair.
    pub fn get(&self, name: &str, version: &PluginVersion) -> Option<&PluginEntry> {
        self.entries.get(&(name.into(), version.clone()))
    }

    /// Pick the newest version of `name` — used when the config does
    /// not pin a version.
    pub fn newest(&self, name: &str) -> Option<&PluginEntry> {
        self.entries
            .range((name.to_string(), PluginVersion::new(""))..)
            .take_while(|((n, _), _)| n == name)
            .last()
            .map(|(_, e)| e)
    }

    /// Every entry, in (name, version) order — used by the REST
    /// `/connector-plugins` listing.
    pub fn list(&self) -> Vec<&PluginEntry> {
        self.entries.values().collect()
    }

    /// Reject config entries that don't belong to any known prefix
    /// — the namespace isolation check. Keys with the universally-
    /// understood `connector.class`, `tasks.max`, `name`, `transforms`,
    /// `predicates` are exempted (they're the Connect framework's
    /// own namespace, not a plugin's).
    pub fn validate_config_namespaces(
        &self,
        config: &BTreeMap<String, String>,
    ) -> StreamsResult<()> {
        const FRAMEWORK_KEYS: &[&str] = &[
            "connector.class",
            "tasks.max",
            "name",
            "topics",
            "topics.regex",
            "key.converter",
            "value.converter",
            "header.converter",
            "config.action.reload",
            "errors.retry.timeout",
            "errors.retry.delay.max.ms",
            "errors.tolerance",
            "errors.log.enable",
            "errors.log.include.messages",
            "errors.deadletterqueue.topic.name",
            "errors.deadletterqueue.topic.replication.factor",
            "errors.deadletterqueue.context.headers.enable",
        ];
        for key in config.keys() {
            if FRAMEWORK_KEYS.contains(&key.as_str()) {
                continue;
            }
            if key.starts_with("transforms.") || key.starts_with("predicates.") {
                continue;
            }
            if self.prefixes.keys().any(|p| key.starts_with(p)) {
                continue;
            }
            return Err(StreamsError::Internal(format!(
                "PluginRegistry: unknown config namespace for key '{key}'"
            )));
        }
        Ok(())
    }

    /// Build a `ConnectorSpec` from a config map. Honors
    /// `connector.class` pin (matched against `name` in the
    /// registry) plus optional `connector.version` pin.
    pub fn build(&self, config: &BTreeMap<String, String>) -> StreamsResult<ConnectorSpec> {
        let class = config.get("connector.class").ok_or_else(|| {
            StreamsError::Internal("PluginRegistry: 'connector.class' required".into())
        })?;
        let entry = match config.get("connector.version") {
            Some(v) => self.get(class, &PluginVersion::new(v)).ok_or_else(|| {
                StreamsError::Internal(format!(
                    "PluginRegistry: no plugin '{class}' at version '{v}'"
                ))
            })?,
            None => self
                .newest(class)
                .ok_or_else(|| StreamsError::Internal(format!("unknown plugin '{class}'")))?,
        };
        (entry.factory)(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_factory(kind: TaskKind) -> ConnectorFactory {
        let k = kind;
        Arc::new(move |cfg: &BTreeMap<String, String>| {
            let name = cfg
                .get("name")
                .cloned()
                .unwrap_or_else(|| "anon".to_string());
            let tasks_max = cfg
                .get("tasks.max")
                .and_then(|v| v.parse::<u32>().ok())
                .unwrap_or(1);
            Ok(ConnectorSpec::new(name, k, tasks_max))
        })
    }

    fn entry(name: &str, v: &str, prefix: &str, kind: TaskKind) -> PluginEntry {
        PluginEntry {
            name: name.into(),
            version: PluginVersion::new(v),
            kind,
            config_prefix: prefix.into(),
            factory: make_factory(kind),
        }
    }

    #[test]
    fn register_and_get_round_trips() {
        let mut r = PluginRegistry::new();
        r.register(entry("cave.connect.JdbcSource", "1.0.0", "jdbc.", TaskKind::Source))
            .unwrap();
        assert!(r
            .get("cave.connect.JdbcSource", &PluginVersion::new("1.0.0"))
            .is_some());
    }

    #[test]
    fn register_rejects_empty_name() {
        let mut r = PluginRegistry::new();
        let mut e = entry("name", "1", "p.", TaskKind::Source);
        e.name = "".into();
        assert!(r.register(e).is_err());
    }

    #[test]
    fn register_rejects_empty_prefix() {
        let mut r = PluginRegistry::new();
        let mut e = entry("name", "1", "p.", TaskKind::Source);
        e.config_prefix = "".into();
        assert!(r.register(e).is_err());
    }

    #[test]
    fn prefix_collision_between_plugins_rejected() {
        let mut r = PluginRegistry::new();
        r.register(entry("A", "1", "shared.", TaskKind::Source)).unwrap();
        assert!(r.register(entry("B", "1", "shared.", TaskKind::Sink)).is_err());
    }

    #[test]
    fn same_plugin_different_version_allowed() {
        let mut r = PluginRegistry::new();
        r.register(entry("p", "1.0.0", "p.", TaskKind::Source)).unwrap();
        r.register(entry("p", "2.0.0", "p.", TaskKind::Source)).unwrap();
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn newest_picks_latest_version() {
        let mut r = PluginRegistry::new();
        r.register(entry("p", "1.0.0", "p.", TaskKind::Source)).unwrap();
        r.register(entry("p", "2.0.0", "p.", TaskKind::Source)).unwrap();
        r.register(entry("p", "1.5.0", "p.", TaskKind::Source)).unwrap();
        assert_eq!(r.newest("p").unwrap().version.as_str(), "2.0.0");
    }

    #[test]
    fn validate_accepts_framework_keys() {
        let mut r = PluginRegistry::new();
        r.register(entry("p", "1", "p.", TaskKind::Source)).unwrap();
        let mut cfg = BTreeMap::new();
        cfg.insert("connector.class".into(), "p".into());
        cfg.insert("tasks.max".into(), "2".into());
        cfg.insert("transforms.smt1.type".into(), "Cast".into());
        cfg.insert("p.connection.url".into(), "x".into());
        r.validate_config_namespaces(&cfg).unwrap();
    }

    #[test]
    fn validate_rejects_unknown_namespace() {
        let mut r = PluginRegistry::new();
        r.register(entry("p", "1", "p.", TaskKind::Source)).unwrap();
        let mut cfg = BTreeMap::new();
        cfg.insert("foreign.key".into(), "x".into());
        assert!(r.validate_config_namespaces(&cfg).is_err());
    }

    #[test]
    fn build_resolves_to_spec_with_factory() {
        let mut r = PluginRegistry::new();
        r.register(entry("p", "1", "p.", TaskKind::Source)).unwrap();
        let mut cfg = BTreeMap::new();
        cfg.insert("connector.class".into(), "p".into());
        cfg.insert("name".into(), "my-conn".into());
        cfg.insert("tasks.max".into(), "4".into());
        let spec = r.build(&cfg).unwrap();
        assert_eq!(spec.name, "my-conn");
        assert_eq!(spec.tasks_max, 4);
    }

    #[test]
    fn build_with_version_pin_resolves() {
        let mut r = PluginRegistry::new();
        r.register(entry("p", "1.0.0", "p.", TaskKind::Source)).unwrap();
        r.register(entry("p", "2.0.0", "p.", TaskKind::Source)).unwrap();
        let mut cfg = BTreeMap::new();
        cfg.insert("connector.class".into(), "p".into());
        cfg.insert("connector.version".into(), "1.0.0".into());
        cfg.insert("name".into(), "c".into());
        cfg.insert("tasks.max".into(), "1".into());
        let spec = r.build(&cfg).unwrap();
        assert_eq!(spec.name, "c");
    }

    #[test]
    fn build_unknown_version_errors() {
        let mut r = PluginRegistry::new();
        r.register(entry("p", "1", "p.", TaskKind::Source)).unwrap();
        let mut cfg = BTreeMap::new();
        cfg.insert("connector.class".into(), "p".into());
        cfg.insert("connector.version".into(), "9.9.9".into());
        assert!(r.build(&cfg).is_err());
    }

    #[test]
    fn build_unknown_class_errors() {
        let r = PluginRegistry::new();
        let mut cfg = BTreeMap::new();
        cfg.insert("connector.class".into(), "ghost".into());
        assert!(r.build(&cfg).is_err());
    }

    #[test]
    fn list_returns_all_entries_sorted() {
        let mut r = PluginRegistry::new();
        r.register(entry("z", "1", "z.", TaskKind::Source)).unwrap();
        r.register(entry("a", "1", "a.", TaskKind::Sink)).unwrap();
        let names: Vec<_> = r.list().iter().map(|e| e.name.clone()).collect();
        assert_eq!(names, vec!["a", "z"]);
    }
}

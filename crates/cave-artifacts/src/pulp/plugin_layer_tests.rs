// SPDX-License-Identifier: AGPL-3.0-or-later
//
// Phase 2 plugin-layer integration tests.
//
// Exercises the registry surface (`PluginRegistry::default()`) wiring all nine
// upstream Pulp content plugins, and asserts plugin/content-type identity for
// each. These are intentionally end-to-end against the wired registry, not the
// individual plugin unit tests (those live in `plugins/*.rs`).

#![cfg(test)]

use crate::pulp::models::PluginType;
use crate::pulp::plugin::PluginRegistry;

#[test]
fn registry_default_registers_nine_upstream_plugins() {
    let registry = PluginRegistry::default();
    let types = registry.list();
    // RPM, Container, Python, File, Deb, Maven, Ansible, OSTree, Helm
    assert_eq!(types.len(), 9, "expected 9 plugins, got {types:?}");
    for required in [
        PluginType::Rpm,
        PluginType::Container,
        PluginType::Python,
        PluginType::File,
        PluginType::Deb,
        PluginType::Maven,
        PluginType::Ansible,
        PluginType::OSTree,
        PluginType::Helm,
    ] {
        assert!(
            registry.get(&required).is_some(),
            "{required:?} missing from registry"
        );
    }
}

#[test]
fn each_plugin_has_unique_name() {
    let registry = PluginRegistry::default();
    let types = registry.list();
    let mut names: Vec<String> = types
        .iter()
        .map(|t| registry.get(t).unwrap().name().to_string())
        .collect();
    names.sort();
    let unique = {
        let mut copy = names.clone();
        copy.dedup();
        copy.len()
    };
    assert_eq!(unique, names.len(), "duplicate plugin names: {names:?}");
}

#[test]
fn each_plugin_advertises_at_least_one_content_type() {
    let registry = PluginRegistry::default();
    for ty in registry.list() {
        let plugin = registry.get(&ty).unwrap();
        assert!(
            !plugin.content_types().is_empty(),
            "{} advertises no content types",
            plugin.name()
        );
    }
}

#[test]
fn pulp_upstream_plugin_names_match_canonical() {
    let registry = PluginRegistry::default();
    // Canonical Pulp upstream package names — every plugin maps to one.
    let expected = [
        (PluginType::Rpm, "pulp_rpm"),
        (PluginType::Container, "pulp_container"),
        (PluginType::Python, "pulp_python"),
        (PluginType::File, "pulp_file"),
        (PluginType::Deb, "pulp_deb"),
        (PluginType::Maven, "pulp_maven"),
        (PluginType::Ansible, "pulp_ansible"),
        (PluginType::OSTree, "pulp_ostree"),
        (PluginType::Helm, "pulp_helm"),
    ];
    for (ty, name) in expected {
        let plugin = registry.get(&ty).unwrap();
        assert_eq!(plugin.name(), name, "{ty:?} -> {}", plugin.name());
    }
}

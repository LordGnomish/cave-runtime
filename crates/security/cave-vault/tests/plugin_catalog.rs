// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Strict-TDD port of openbao `vault/plugin_catalog.go` + `sdk/helper/consts/plugin_types.go`
// (pinned v2.5.4, source_sha 4f6d47246a053375271a5fd8af85c3b75695aa46).
//
// Scope: the pure registry/data layer — Set / Get / List / Delete, sha256 hex
// validation, parent-reference guard, type-namespaced storage keys, builtin
// fallback, version selection. The external-process runner + go-plugin gRPC
// multiplexing stays a documented scope_cut (no subprocess exec in-crate).

use cave_vault::plugins::{PluginCatalog, PluginError, PluginType, SetPluginInput};

// ── Cycle 1: storage-key namespacing + set/get/delete + sha256 + parent-ref ──

fn valid_input(name: &str, t: PluginType) -> SetPluginInput {
    SetPluginInput {
        name: name.to_string(),
        plugin_type: t,
        version: String::new(),
        command: format!("{name}-plugin"),
        args: vec![],
        env: vec![],
        // 32-byte (64 hex char) digest — a real sha256 length.
        sha256_hex: "a".repeat(64),
        oci: false,
    }
}

#[test]
fn set_then_get_round_trips() {
    let mut cat = PluginCatalog::new();
    cat.set(valid_input("mysql", PluginType::Database)).unwrap();
    let got = cat
        .get("mysql", PluginType::Database, "")
        .expect("registered plugin must be retrievable");
    assert_eq!(got.name, "mysql");
    assert_eq!(got.plugin_type, PluginType::Database);
    assert_eq!(got.command, "mysql-plugin");
    assert!(!got.builtin, "externally-registered plugin is not builtin");
    assert_eq!(got.sha256.len(), 32, "64 hex chars decode to 32 bytes");
}

#[test]
fn get_unknown_returns_none() {
    let cat = PluginCatalog::new();
    assert!(cat.get("ghost", PluginType::Secrets, "").is_none());
}

#[test]
fn set_rejects_parent_reference_in_name() {
    let mut cat = PluginCatalog::new();
    let mut input = valid_input("../evil", PluginType::Secrets);
    input.name = "../evil".to_string();
    assert_eq!(
        cat.set(input),
        Err(PluginError::PathContainsParentReferences)
    );
}

#[test]
fn set_rejects_parent_reference_in_command() {
    let mut cat = PluginCatalog::new();
    let mut input = valid_input("ok", PluginType::Secrets);
    input.command = "../../bin/sh".to_string();
    assert_eq!(
        cat.set(input),
        Err(PluginError::PathContainsParentReferences)
    );
}

#[test]
fn set_rejects_non_hex_sha256() {
    let mut cat = PluginCatalog::new();
    let mut input = valid_input("ok", PluginType::Secrets);
    input.sha256_hex = "zzzz".to_string();
    assert!(matches!(cat.set(input), Err(PluginError::InvalidSha256(_))));
}

#[test]
fn set_rejects_too_short_sha256() {
    // Upstream requires a minimum of 8 hex chars (4 raw bytes).
    let mut cat = PluginCatalog::new();
    let mut input = valid_input("ok", PluginType::Secrets);
    input.sha256_hex = "abcd".to_string(); // 2 bytes < 4
    assert!(matches!(cat.set(input), Err(PluginError::InvalidSha256(_))));
}

#[test]
fn storage_key_namespaces_by_type() {
    // Same name under two types must not collide.
    let mut cat = PluginCatalog::new();
    cat.set(valid_input("dup", PluginType::Database)).unwrap();
    cat.set(valid_input("dup", PluginType::Secrets)).unwrap();
    assert!(cat.get("dup", PluginType::Database, "").is_some());
    assert!(cat.get("dup", PluginType::Secrets, "").is_some());
    // The internal key reflects the type segment.
    assert_eq!(
        PluginCatalog::storage_key(PluginType::Database, "dup", ""),
        "database/dup"
    );
    assert_eq!(
        PluginCatalog::storage_key(PluginType::Secrets, "dup", ""),
        "secret/dup"
    );
}

#[test]
fn delete_removes_external_entry() {
    let mut cat = PluginCatalog::new();
    cat.set(valid_input("temp", PluginType::Secrets)).unwrap();
    let removed = cat.delete("temp", PluginType::Secrets, "");
    assert!(removed.is_some());
    assert!(cat.get("temp", PluginType::Secrets, "").is_none());
    // Deleting again is a no-op.
    assert!(cat.delete("temp", PluginType::Secrets, "").is_none());
}

// ── Cycle 2: builtin registry fallback + list merge/dedup/sort + shadowing ──

#[test]
fn get_falls_back_to_builtin_when_not_external() {
    let mut cat = PluginCatalog::new();
    cat.register_builtin("kubernetes", PluginType::Credential);
    let got = cat
        .get("kubernetes", PluginType::Credential, "")
        .expect("builtin must resolve");
    assert!(got.builtin);
    assert_eq!(got.name, "kubernetes");
    assert!(got.command.is_empty(), "builtins have no exec command");
}

#[test]
fn external_unversioned_shadows_builtin() {
    // An external registration of the same name+type takes precedence over the
    // builtin (upstream: "Unversioned external plugins shadow builtins").
    let mut cat = PluginCatalog::new();
    cat.register_builtin("postgresql", PluginType::Database);
    cat.set(valid_input("postgresql", PluginType::Database))
        .unwrap();
    let got = cat.get("postgresql", PluginType::Database, "").unwrap();
    assert!(!got.builtin, "external registration shadows the builtin");
    assert_eq!(got.command, "postgresql-plugin");
}

#[test]
fn list_merges_builtin_and_external_sorted_deduped() {
    let mut cat = PluginCatalog::new();
    cat.register_builtin("aws", PluginType::Secrets);
    cat.register_builtin("kv", PluginType::Secrets);
    cat.set(valid_input("kv", PluginType::Secrets)).unwrap(); // dup name
    cat.set(valid_input("transit", PluginType::Secrets)).unwrap();
    let names = cat.list(PluginType::Secrets);
    // Sorted, de-duplicated union of builtin + external names.
    assert_eq!(names, vec!["aws", "kv", "transit"]);
}

#[test]
fn list_scopes_by_type() {
    let mut cat = PluginCatalog::new();
    cat.set(valid_input("mysql", PluginType::Database)).unwrap();
    cat.set(valid_input("ldap", PluginType::Credential)).unwrap();
    assert_eq!(cat.list(PluginType::Database), vec!["mysql"]);
    assert_eq!(cat.list(PluginType::Credential), vec!["ldap"]);
    assert!(cat.list(PluginType::Secrets).is_empty());
}

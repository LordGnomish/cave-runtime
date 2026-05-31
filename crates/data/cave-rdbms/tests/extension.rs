// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Strict-TDD port of PostgreSQL's extension framework
// (src/backend/commands/extension.c + the .control file format):
//   * parse_extension_control_file — key = 'value' / key = bool parsing of
//     default_version / comment / requires / relocatable / schema / trusted
//   * CREATE EXTENSION — record into the pg_extension catalog, refusing to
//     install when a `requires` dependency is absent, or on a re-install
//   * DROP EXTENSION — refused while another installed extension depends on it
//   * dependency-ordered install (topological over `requires`)

use cave_rdbms::storage::extension::{
    parse_control, ExtError, ExtensionControl, ExtensionRegistry,
};

const VECTOR_CONTROL: &str = "\
# vector extension
default_version = '1.3'
comment = 'vector data type and ivfflat/hnsw access methods'
relocatable = true
trusted = true
requires = 'plpgsql'
";

#[test]
fn parses_control_file() {
    let c = parse_control("vector", VECTOR_CONTROL);
    assert_eq!(c.name, "vector");
    assert_eq!(c.default_version, "1.3");
    assert_eq!(c.comment, "vector data type and ivfflat/hnsw access methods");
    assert!(c.relocatable);
    assert!(c.trusted);
    assert_eq!(c.requires, vec!["plpgsql".to_string()]);
}

#[test]
fn parses_multi_requires_and_defaults() {
    let c = parse_control("postgis", "default_version = '3.4'\nrequires = 'plpgsql, fuzzystrmatch'\n");
    assert_eq!(c.default_version, "3.4");
    assert_eq!(c.requires, vec!["plpgsql".to_string(), "fuzzystrmatch".to_string()]);
    // unspecified booleans default to false
    assert!(!c.relocatable);
    assert!(!c.trusted);
}

#[test]
fn create_extension_enforces_dependencies() {
    let mut reg = ExtensionRegistry::new();
    let plpgsql = ExtensionControl::bare("plpgsql", "1.0");
    let vector = parse_control("vector", VECTOR_CONTROL);

    // vector requires plpgsql, which is not installed yet
    assert_eq!(
        reg.create(&vector),
        Err(ExtError::MissingDependency("plpgsql".into()))
    );

    reg.create(&plpgsql).unwrap();
    reg.create(&vector).unwrap();
    assert_eq!(reg.installed_version("vector"), Some("1.3".to_string()));

    // re-installing is rejected
    assert_eq!(reg.create(&vector), Err(ExtError::AlreadyInstalled));
}

#[test]
fn drop_extension_blocks_on_dependents() {
    let mut reg = ExtensionRegistry::new();
    reg.create(&ExtensionControl::bare("plpgsql", "1.0")).unwrap();
    reg.create(&parse_control("vector", VECTOR_CONTROL)).unwrap();

    // plpgsql still has a dependent (vector)
    assert_eq!(
        reg.drop("plpgsql"),
        Err(ExtError::DependencyExists("vector".into()))
    );
    // dropping the leaf first, then the dependency, both succeed
    reg.drop("vector").unwrap();
    reg.drop("plpgsql").unwrap();
    assert_eq!(reg.installed_version("plpgsql"), None);

    assert_eq!(reg.drop("ghost"), Err(ExtError::NotFound));
}

#[test]
fn topological_install_order() {
    // c requires b, b requires a → install order a, b, c
    let a = ExtensionControl::bare("a", "1");
    let mut b = ExtensionControl::bare("b", "1");
    b.requires = vec!["a".into()];
    let mut c = ExtensionControl::bare("c", "1");
    c.requires = vec!["b".into()];

    let order = ExtensionRegistry::install_order(&[c.clone(), a.clone(), b.clone()]).unwrap();
    let names: Vec<&str> = order.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["a", "b", "c"]);
}

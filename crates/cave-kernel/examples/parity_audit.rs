// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Workspace parity audit: prints one CSV row per crate.
//!
//! Columns: name,has_manifest,upstream_ref,file_m,file_t,fn_m,fn_t,test_m,test_t,
//!          surf_m,surf_t,overall,stubs,lib_lines
//!
//! Run from the workspace root:
//!   cargo run -p cave-kernel --example parity_audit > /tmp/audit.csv

use cave_kernel::parity::{DiscoveredReport, discover_workspace};
use std::fs;
use std::path::Path;

fn main() {
    let root = std::env::current_dir().unwrap();
    let crates_dir = root.join("crates");

    let mut by_name: std::collections::HashMap<String, DiscoveredReport> =
        std::collections::HashMap::new();
    for d in discover_workspace(&root) {
        // discover_workspace keys by manifest module.name, but we want crate dir name
        // for cross-checking. Recover the crate dir from manifest_path.
        let crate_dir = d
            .manifest_path
            .parent()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        by_name.insert(crate_dir, d);
    }

    println!(
        "name,has_manifest,upstream_ref,file_m,file_t,fn_m,fn_t,test_m,test_t,surf_m,surf_t,overall,stubs,lib_lines"
    );

    let mut entries: Vec<_> = fs::read_dir(&crates_dir)
        .unwrap()
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    entries.sort();

    for name in entries {
        let crate_root = crates_dir.join(&name);
        let lib_lines = lib_line_count(&crate_root);
        match by_name.get(&name) {
            Some(d) => {
                let r = &d.report;
                println!(
                    "{},yes,{},{},{},{},{},{},{},{},{},{:.4},{},{}",
                    name,
                    r.upstream_ref.replace(',', ";"),
                    r.file_parity.matched,
                    r.file_parity.total,
                    r.function_parity.matched,
                    r.function_parity.total,
                    r.test_parity.matched,
                    r.test_parity.total,
                    r.surface_parity.matched,
                    r.surface_parity.total,
                    r.overall,
                    r.stubs_detected,
                    lib_lines
                );
            }
            None => {
                println!("{},no,,,,,,,,,,,,{}", name, lib_lines);
            }
        }
    }
}

fn lib_line_count(crate_root: &Path) -> usize {
    let lib = crate_root.join("src").join("lib.rs");
    let main = crate_root.join("src").join("main.rs");
    let path = if lib.exists() {
        lib
    } else if main.exists() {
        main
    } else {
        return 0;
    };
    fs::read_to_string(&path)
        .map(|s| s.lines().count())
        .unwrap_or(0)
}

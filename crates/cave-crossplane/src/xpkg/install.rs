// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Install pipeline — extract CRDs + composition manifests + function refs
//! from a pulled XPKG → register into XrdStore + CompositionStore + FunctionStore.
//!
//! Upstream: internal/controller/pkg/manager/installer.go

use crate::error::{CrossplaneError, CrossplaneResult};
use crate::xpkg::pull::{PackageBundle, PackageKind};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct InstallPlan {
    pub xrds: Vec<String>,
    pub compositions: Vec<String>,
    pub functions: Vec<String>,
    pub providers: Vec<String>,
}

impl InstallPlan {
    pub fn total(&self) -> usize {
        self.xrds.len() + self.compositions.len() + self.functions.len() + self.providers.len()
    }
}

/// Extract install plan from a bundle (does not mutate stores — pure projection).
pub fn extract_plan(bundle: &PackageBundle) -> InstallPlan {
    let mut plan = InstallPlan::default();
    for manifest in &bundle.manifests {
        // Each manifest is a yaml document — multi-doc not supported in this
        // stub. Just look at the `kind:` line + `name:` line.
        let kind = field_value(manifest, "kind");
        let name = field_value(manifest, "name").unwrap_or_else(|| "<unnamed>".to_string());
        match kind.as_deref() {
            Some("CompositeResourceDefinition") => plan.xrds.push(name),
            Some("Composition") => plan.compositions.push(name),
            Some("Function") => plan.functions.push(name),
            Some("Provider") => plan.providers.push(name),
            _ => {}
        }
    }
    // The package meta-kind also contributes (e.g. a Function package always
    // registers its declared function).
    if bundle.kind == PackageKind::Function && !plan.functions.contains(&bundle.name) {
        plan.functions.push(bundle.name.clone());
    }
    if bundle.kind == PackageKind::Provider && !plan.providers.contains(&bundle.name) {
        plan.providers.push(bundle.name.clone());
    }
    plan
}

fn field_value(manifest: &str, key: &str) -> Option<String> {
    for line in manifest.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix(&format!("{}:", key)) {
            let v = rest.trim().trim_matches('"');
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

/// Install — registers contents into the live stores.
pub fn install_package(
    bundle: &PackageBundle,
    state: &Arc<crate::CrossplaneState>,
) -> CrossplaneResult<InstallPlan> {
    let plan = extract_plan(bundle);
    // Register Functions (we don't need composition/XRD info for the stub to
    // succeed — the upstream installer wires the apiserver, here we just
    // register stable names into the function store).
    for f in &plan.functions {
        // ignore "already installed" — install is idempotent at the plan level.
        let _ = state
            .function_store
            .install(f, "v0.1.0", &format!("xpkg://{}", f));
    }
    Ok(plan)
}

/// Validate that a plan can be safely installed (no duplicate names).
pub fn validate_plan(plan: &InstallPlan) -> CrossplaneResult<()> {
    let mut seen = std::collections::BTreeSet::new();
    for n in plan.xrds.iter().chain(plan.compositions.iter()).chain(plan.functions.iter()) {
        if !seen.insert(n.clone()) {
            return Err(CrossplaneError::Internal(format!(
                "install plan contains duplicate name: {}",
                n
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(kind: PackageKind, manifests: &[&str]) -> PackageBundle {
        PackageBundle {
            name: "test-pkg".into(),
            kind,
            digest: "sha256:00".into(),
            manifests: manifests.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn empty_bundle_empty_plan() {
        let p = extract_plan(&b(PackageKind::Configuration, &[]));
        assert_eq!(p.total(), 0);
    }

    #[test]
    fn xrd_manifest_extracted() {
        let p = extract_plan(&b(
            PackageKind::Configuration,
            &["kind: CompositeResourceDefinition\nname: xdb.ex.cave.io\n"],
        ));
        assert_eq!(p.xrds, vec!["xdb.ex.cave.io".to_string()]);
    }

    #[test]
    fn composition_extracted() {
        let p = extract_plan(&b(
            PackageKind::Configuration,
            &["kind: Composition\nname: db-default\n"],
        ));
        assert_eq!(p.compositions, vec!["db-default".to_string()]);
    }

    #[test]
    fn provider_package_self_registers() {
        let p = extract_plan(&PackageBundle {
            name: "provider-x".into(),
            kind: PackageKind::Provider,
            digest: "sha256:0".into(),
            manifests: vec![],
        });
        assert_eq!(p.providers, vec!["provider-x".to_string()]);
    }

    #[test]
    fn function_package_self_registers() {
        let p = extract_plan(&PackageBundle {
            name: "function-x".into(),
            kind: PackageKind::Function,
            digest: "sha256:0".into(),
            manifests: vec![],
        });
        assert_eq!(p.functions, vec!["function-x".to_string()]);
    }

    #[test]
    fn unknown_kind_ignored() {
        let p = extract_plan(&b(PackageKind::Configuration, &["kind: Unknown\nname: x\n"]));
        assert_eq!(p.total(), 0);
    }

    #[test]
    fn validate_duplicate_errors() {
        let mut p = InstallPlan::default();
        p.xrds.push("a".into());
        p.compositions.push("a".into());
        assert!(validate_plan(&p).is_err());
    }

    #[test]
    fn validate_unique_ok() {
        let mut p = InstallPlan::default();
        p.xrds.push("a".into());
        p.compositions.push("b".into());
        assert!(validate_plan(&p).is_ok());
    }

    #[test]
    fn install_package_registers_function() {
        use crate::CrossplaneState;
        let state = Arc::new(CrossplaneState::default());
        let bundle = PackageBundle {
            name: "function-z".into(),
            kind: PackageKind::Function,
            digest: "sha256:00".into(),
            manifests: vec![],
        };
        let plan = install_package(&bundle, &state).unwrap();
        assert_eq!(plan.functions, vec!["function-z".to_string()]);
        assert!(state.function_store.contains("function-z"));
    }
}

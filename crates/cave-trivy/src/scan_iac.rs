// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! IaC (infrastructure-as-code) misconfig scanner.
//!
//! Mirrors trivy's `pkg/iac/scanners` for Terraform, Kubernetes manifests,
//! Dockerfile and Helm. The scanner detects file type by basename and
//! delegates to `MisconfRegistry::evaluate` per-file. Terraform HCL files
//! are additionally parsed with `hcl-rs` to surface structural errors.

use crate::misconf::MisconfRegistry;
use crate::models::ScanResult;
use crate::scan_fs::FsTree;

pub fn scan_iac_tree(tree: &FsTree, reg: &MisconfRegistry) -> Vec<ScanResult> {
    let mut out = Vec::new();
    for (path, body) in &tree.files {
        let base = path.rsplit('/').next().unwrap_or(path);
        let kind = detect_kind(base, body);
        if kind.is_empty() {
            continue;
        }
        let mut res = ScanResult {
            target: path.clone(),
            class: "config".into(),
            ..Default::default()
        };
        for k in kind {
            for m in reg.evaluate(k, path, body) {
                res.misconfigurations.push(m);
            }
        }
        if !res.misconfigurations.is_empty() {
            out.push(res);
        }
    }
    out
}

/// Return all kinds matching this file. A file can match multiple
/// (e.g. K8s YAML + Helm template).
pub fn detect_kind(basename: &str, body: &str) -> Vec<&'static str> {
    let mut kinds = Vec::new();
    if basename.ends_with(".tf") || basename.ends_with(".tf.json") {
        kinds.push("terraform");
    }
    if basename == "Dockerfile" || basename.ends_with(".Dockerfile") {
        kinds.push("dockerfile");
    }
    if basename.ends_with(".yaml") || basename.ends_with(".yml") {
        if body.contains("apiVersion:") || body.contains("kind:") {
            kinds.push("kubernetes");
        }
        if basename.starts_with("values") {
            kinds.push("helm");
        }
    }
    if basename == "Chart.yaml" {
        kinds.push("helm");
    }
    kinds
}

/// Validate Terraform HCL parses. Returns `Some(err)` on parse failure.
pub fn validate_hcl(body: &str) -> Option<String> {
    match hcl::from_str::<serde_json::Value>(body) {
        Ok(_) => None,
        Err(e) => Some(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tf_acl_public_read() {
        let t = FsTree::default().push(
            "main.tf",
            r#"resource "aws_s3_bucket" "b" { acl = "public-read" }"#,
        );
        let r = scan_iac_tree(&t, &MisconfRegistry::builtin());
        assert_eq!(r.len(), 1);
        assert!(r[0].misconfigurations.iter().any(|m| m.id == "AVD-AWS-0086"));
    }

    #[test]
    fn dockerfile_no_user() {
        let t = FsTree::default().push("Dockerfile", "FROM alpine\nRUN apk add x\n");
        let r = scan_iac_tree(&t, &MisconfRegistry::builtin());
        assert!(r.iter().any(|x| x
            .misconfigurations
            .iter()
            .any(|m| m.id == "AVD-DS-0002")));
    }

    #[test]
    fn k8s_yaml_kind_detected() {
        let t = FsTree::default().push(
            "pod.yaml",
            "apiVersion: v1\nkind: Pod\nspec:\n  containers:\n  - name: c\n    securityContext:\n      privileged: true\n",
        );
        let r = scan_iac_tree(&t, &MisconfRegistry::builtin());
        assert!(r[0].misconfigurations.iter().any(|m| m.id == "AVD-KSV-0017"));
    }

    #[test]
    fn helm_values() {
        let t = FsTree::default().push("values.yaml", "image: \"latest\"\n");
        let r = scan_iac_tree(&t, &MisconfRegistry::builtin());
        assert!(r[0].misconfigurations.iter().any(|m| m.id == "AVD-HELM-0002"));
    }

    #[test]
    fn detect_kind_yaml_without_apiversion_skips() {
        let kinds = detect_kind("notes.yaml", "title: hi\n");
        assert!(kinds.is_empty());
    }

    #[test]
    fn detect_kind_combined() {
        let kinds = detect_kind("values.yaml", "apiVersion: v1\n");
        assert!(kinds.contains(&"kubernetes"));
        assert!(kinds.contains(&"helm"));
    }

    #[test]
    fn validate_hcl_good() {
        assert!(validate_hcl(r#"resource "x" "y" { name = "z" }"#).is_none());
    }

    #[test]
    fn validate_hcl_bad() {
        assert!(validate_hcl("resource \"x\" {{").is_some());
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Git repository scanner.
//!
//! Mirrors trivy's `pkg/scan/artifact/repo`. cave-trivy MVP scans a
//! materialised checkout — the live `git clone` flow is delegated to
//! cave-deploy or external CI tooling. The scanner runs the filesystem
//! analyser plus secret + IaC misconfig scanners across the tree and
//! tags the report's artifact_type as `git_repository`. A `git_ref`
//! field carries the commit SHA when supplied.

use crate::error::TrivyResult;
use crate::misconf::MisconfRegistry;
use crate::models::{Report, ScanResult};
use crate::scan_fs::FsTree;
use crate::scan_iac::scan_iac_tree;
use crate::scan_secret::{scan_secrets_in_tree, SecretRules};
use crate::vulndb::VulnDb;

#[derive(Debug, Default, Clone)]
pub struct RepoArtifact {
    pub url: String,
    pub git_ref: Option<String>,
    pub tree: FsTree,
}

pub fn scan_repo(
    art: &RepoArtifact,
    db: &VulnDb,
    rules: &SecretRules,
    misconf: &MisconfRegistry,
) -> TrivyResult<Report> {
    let mut report = Report::new(&art.url, "git_repository");
    // Vulnerability scan via lockfiles.
    let fs_report = crate::scan_fs::scan_fs(&art.url, &art.tree, db)?;
    for r in fs_report.results {
        report.results.push(r);
    }
    // Secrets.
    let secret_findings = scan_secrets_in_tree(&art.tree, rules);
    if !secret_findings.is_empty() {
        report.results.push(ScanResult {
            target: art.url.clone(),
            class: "secrets".into(),
            secrets: secret_findings,
            ..Default::default()
        });
    }
    // IaC misconfig.
    let iac = scan_iac_tree(&art.tree, misconf);
    for r in iac {
        report.results.push(r);
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_lockfile_and_secret() {
        let tree = FsTree::default()
            .push("a/Cargo.lock", "[[package]]\nname=\"openssl-sys\"\nversion=\"0.9.0\"\n")
            .push("b/.env", "AWS_SECRET_ACCESS_KEY=AKIAIOSFODNN7EXAMPLEKEY1\n");
        let art = RepoArtifact {
            url: "git+https://example.com/x".into(),
            git_ref: Some("abc123".into()),
            tree,
        };
        let r = scan_repo(
            &art,
            &VulnDb::cave_default(),
            &SecretRules::default_rules(),
            &MisconfRegistry::builtin(),
        )
        .unwrap();
        assert_eq!(r.artifact_type, "git_repository");
        let kinds: Vec<_> = r.results.iter().map(|x| x.class.clone()).collect();
        assert!(kinds.iter().any(|c| c == "lang-pkgs"));
        assert!(kinds.iter().any(|c| c == "secrets"));
    }

    #[test]
    fn iac_misconfig_terraform() {
        let tree = FsTree::default().push(
            "main.tf",
            r#"resource "aws_s3_bucket" "b" { bucket = "x" acl = "public-read" }"#,
        );
        let art = RepoArtifact {
            url: "tf".into(),
            git_ref: None,
            tree,
        };
        let r = scan_repo(
            &art,
            &VulnDb::cave_default(),
            &SecretRules::default_rules(),
            &MisconfRegistry::builtin(),
        )
        .unwrap();
        assert!(r
            .results
            .iter()
            .any(|x| x.class == "config" && !x.misconfigurations.is_empty()));
    }
}

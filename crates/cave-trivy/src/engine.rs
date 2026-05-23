// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Top-level scan orchestrator.
//!
//! Mirrors trivy's `pkg/scan/Service` for the common "scan + filter +
//! render" path: pick the scanner by target, apply filters, optionally
//! apply VEX, then render with the requested writer.

use crate::error::TrivyResult;
use crate::filter::Filter;
use crate::misconf::MisconfRegistry;
use crate::models::Report;
use crate::scan_fs::FsTree;
use crate::scan_image::{ImageArtifact, ScanImageOpts};
use crate::scan_repo::RepoArtifact;
use crate::scan_secret::SecretRules;
use crate::vex::{VexDocument, VexIndex};
use crate::vulndb::VulnDb;

pub struct Engine {
    pub db: VulnDb,
    pub secrets: SecretRules,
    pub misconf: MisconfRegistry,
    pub vex: Option<VexIndex>,
}

impl Default for Engine {
    fn default() -> Self {
        Self {
            db: VulnDb::cave_default(),
            secrets: SecretRules::default_rules(),
            misconf: MisconfRegistry::builtin(),
            vex: None,
        }
    }
}

impl Engine {
    pub fn new(
        db: VulnDb,
        secrets: SecretRules,
        misconf: MisconfRegistry,
        vex: Option<VexIndex>,
    ) -> Self {
        Self {
            db,
            secrets,
            misconf,
            vex,
        }
    }

    pub fn with_vex_document(mut self, doc: &VexDocument) -> Self {
        self.vex = Some(VexIndex::from_document(doc));
        self
    }

    pub fn scan_image(&self, art: &ImageArtifact, opts: ScanImageOpts) -> TrivyResult<Report> {
        let mut r = crate::scan_image::scan_image(art, &self.db, opts)?;
        self.post_process(&mut r);
        Ok(r)
    }

    pub fn scan_fs(&self, name: &str, tree: &FsTree) -> TrivyResult<Report> {
        let mut r = crate::scan_fs::scan_fs(name, tree, &self.db)?;
        self.post_process(&mut r);
        Ok(r)
    }

    pub fn scan_repo(&self, art: &RepoArtifact) -> TrivyResult<Report> {
        let mut r = crate::scan_repo::scan_repo(art, &self.db, &self.secrets, &self.misconf)?;
        self.post_process(&mut r);
        Ok(r)
    }

    pub fn scan_sbom(&self, name: &str, text: &str) -> TrivyResult<Report> {
        let mut r = crate::scan_sbom::scan_sbom(name, text, &self.db)?;
        self.post_process(&mut r);
        Ok(r)
    }

    pub fn scan_k8s(&self, snap: &crate::scan_k8s::K8sClusterSnapshot) -> TrivyResult<Report> {
        let mut r = crate::scan_k8s::scan_cluster(snap, &self.misconf)?;
        self.post_process(&mut r);
        Ok(r)
    }

    pub fn scan_secret(&self, name: &str, tree: &FsTree) -> Report {
        let mut r = crate::scan_secret::scan_secrets_report(name, tree, &self.secrets);
        self.post_process(&mut r);
        r
    }

    pub fn scan_config(&self, name: &str, tree: &FsTree) -> Report {
        let mut r = Report::new(name, "filesystem");
        for s in crate::scan_iac::scan_iac_tree(tree, &self.misconf) {
            r.results.push(s);
        }
        self.post_process(&mut r);
        r
    }

    pub fn filter_and_render(
        &self,
        report: &mut Report,
        filter: &Filter,
        renderer: Renderer,
    ) -> TrivyResult<String> {
        filter.apply(report);
        Ok(match renderer {
            Renderer::Json => crate::report_json::write(report)?,
            Renderer::Table => crate::report_table::write(report),
            Renderer::Sarif => crate::report_sarif::write(report)?,
            Renderer::CycloneDx => crate::sbom_cyclonedx::emit_from_report(report)?,
            Renderer::Spdx => crate::sbom_spdx::emit(&report.artifact_name, &[])?,
            Renderer::Template(t) => crate::report_template::render(report, &t)?,
        })
    }

    fn post_process(&self, r: &mut Report) {
        if let Some(idx) = &self.vex {
            let product = format!("pkg:oci/{}", r.artifact_name);
            for res in &mut r.results {
                crate::vex::apply(idx, &product, res);
            }
        }
    }
}

pub enum Renderer {
    Json,
    Table,
    Sarif,
    CycloneDx,
    Spdx,
    Template(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan_image::ImageArtifact;

    fn fixture_alpine() -> ImageArtifact {
        ImageArtifact {
            name: "alpine:3.19".into(),
            digest: "sha256:aa".into(),
            os_release: Some("ID=alpine\nVERSION_ID=3.19.1".into()),
            apk_db: Some("P:openssl\nV:3.0.0\n".into()),
            ..Default::default()
        }
    }

    #[test]
    fn engine_scan_image_then_filter() {
        let e = Engine::default();
        let mut r = e.scan_image(&fixture_alpine(), ScanImageOpts::default()).unwrap();
        let f = Filter::default().min_severity(crate::severity::Severity::Critical);
        let s = e.filter_and_render(&mut r, &f, Renderer::Json).unwrap();
        assert!(s.contains("CVE-2026-0001"));
    }

    #[test]
    fn engine_vex_suppresses() {
        let mut e = Engine::default();
        let doc = crate::vex::VexDocument {
            context: "".into(),
            statements: vec![crate::vex::VexStatement {
                vulnerability: "CVE-2026-0001".into(),
                products: vec!["pkg:oci/alpine:3.19".into()],
                status: crate::vex::VexStatus::NotAffected,
                justification: None,
            }],
        };
        e = e.with_vex_document(&doc);
        let r = e.scan_image(&fixture_alpine(), ScanImageOpts::default()).unwrap();
        assert!(!r
            .results
            .iter()
            .any(|s| s.vulnerabilities.iter().any(|v| v.id == "CVE-2026-0001")));
    }

    #[test]
    fn engine_scan_fs_runs() {
        let e = Engine::default();
        let tree = FsTree::default().push("Cargo.lock", "[[package]]\nname=\"x\"\nversion=\"1\"\n");
        let r = e.scan_fs("repo", &tree).unwrap();
        assert_eq!(r.artifact_type, "filesystem");
    }

    #[test]
    fn engine_scan_secret_only() {
        let e = Engine::default();
        let tree = FsTree::default().push(".env", "AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE");
        let r = e.scan_secret("repo", &tree);
        assert!(r.total_secrets() >= 1);
    }

    #[test]
    fn engine_render_table() {
        let e = Engine::default();
        let mut r = e.scan_image(&fixture_alpine(), ScanImageOpts::default()).unwrap();
        let s = e
            .filter_and_render(&mut r, &Filter::default(), Renderer::Table)
            .unwrap();
        assert!(s.contains("Artifact:"));
    }

    #[test]
    fn engine_render_sarif() {
        let e = Engine::default();
        let mut r = e.scan_image(&fixture_alpine(), ScanImageOpts::default()).unwrap();
        let s = e
            .filter_and_render(&mut r, &Filter::default(), Renderer::Sarif)
            .unwrap();
        assert!(s.contains("sarif"));
    }

    #[test]
    fn engine_scan_config_only() {
        let e = Engine::default();
        let tree = FsTree::default().push("main.tf", r#"acl = "public-read""#);
        let r = e.scan_config("tf", &tree);
        assert!(r.total_misconfigs() >= 1);
    }
}

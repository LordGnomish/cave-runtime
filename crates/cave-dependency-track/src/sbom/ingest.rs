// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Ingestion engine — uploads a CycloneDX BOM into a project.
//!
//! Mirrors `org.dependencytrack.tasks.BomUploadProcessingTask`.

use super::cyclonedx::{CdxComponent, CycloneDxBom};
use crate::error::Result;
use crate::models::Component;
use crate::portfolio::PortfolioStore;
use uuid::Uuid;

#[derive(Debug, Default, Clone, PartialEq)]
pub struct IngestReport {
    pub project: Uuid,
    pub inserted: usize,
    pub updated: usize,
    pub skipped: usize,
}

pub fn ingest(store: &PortfolioStore, project: Uuid, bom: &CycloneDxBom) -> Result<IngestReport> {
    // Verify project exists.
    let _ = store.get(project)?;
    let mut report = IngestReport {
        project,
        ..Default::default()
    };
    let existing = store.components_for(project);
    for cdx in &bom.components {
        if let Some(c) = build_component(project, cdx) {
            if existing.iter().any(|e| same_identity(e, &c)) {
                report.updated += 1;
                continue;
            }
            store.add_component(c)?;
            report.inserted += 1;
        } else {
            report.skipped += 1;
        }
    }
    Ok(report)
}

fn build_component(project: Uuid, cdx: &CdxComponent) -> Option<Component> {
    if cdx.name.trim().is_empty() {
        return None;
    }
    let mut c = Component::new(project, &cdx.name);
    c.version = cdx.version.clone();
    c.group = cdx.group.clone();
    c.purl = cdx.purl.clone();
    c.cpe = cdx.cpe.clone();
    c.classifier = cdx.classifier();
    for (alg, content) in &cdx.hashes {
        match alg.to_ascii_uppercase().as_str() {
            "MD5" => c.md5 = Some(content.clone()),
            "SHA-1" | "SHA1" => c.sha1 = Some(content.clone()),
            "SHA-256" | "SHA256" => c.sha256 = Some(content.clone()),
            "SHA-512" | "SHA512" => c.sha512 = Some(content.clone()),
            _ => {}
        }
    }
    if let Some(first) = cdx.licenses.first() {
        if first.contains(" OR ") || first.contains(" AND ") || first.contains('(') {
            c.license_expression = Some(first.clone());
        } else {
            c.license = Some(first.clone());
        }
    }
    Some(c)
}

fn same_identity(a: &Component, b: &Component) -> bool {
    if let (Some(p1), Some(p2)) = (&a.purl, &b.purl) {
        if p1 == p2 {
            return true;
        }
    }
    a.name == b.name && a.version == b.version && a.group == b.group
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Classifier, Project};
    use crate::sbom::cyclonedx::parse_cyclonedx_json;

    const BOM: &str = r#"{
        "bomFormat":"CycloneDX","specVersion":"1.6","version":1,
        "components":[
          {"type":"library","name":"serde","version":"1","purl":"pkg:cargo/serde@1",
           "hashes":[{"alg":"SHA-256","content":"AA"}],
           "licenses":[{"license":{"id":"MIT"}}]},
          {"type":"library","name":"tokio","version":"1","licenses":[{"expression":"Apache-2.0 OR MIT"}]},
          {"type":"library","name":""}
        ]}"#;

    #[test]
    fn ingest_inserts_named_components() {
        let s = PortfolioStore::new();
        let p = s.insert(Project::new("cave", Classifier::Application)).unwrap();
        let bom = parse_cyclonedx_json(BOM).unwrap();
        let report = ingest(&s, p.uuid, &bom).unwrap();
        assert_eq!(report.inserted, 2);
        assert_eq!(report.skipped, 1);
        assert_eq!(report.updated, 0);
        let comps = s.components_for(p.uuid);
        assert_eq!(comps.len(), 2);
        let serde_c = comps.iter().find(|c| c.name == "serde").unwrap();
        assert_eq!(serde_c.sha256.as_deref(), Some("AA"));
        assert_eq!(serde_c.license.as_deref(), Some("MIT"));
        let tokio_c = comps.iter().find(|c| c.name == "tokio").unwrap();
        assert_eq!(tokio_c.license_expression.as_deref(), Some("Apache-2.0 OR MIT"));
    }

    #[test]
    fn ingest_dedupes_by_purl() {
        let s = PortfolioStore::new();
        let p = s.insert(Project::new("cave", Classifier::Application)).unwrap();
        let bom = parse_cyclonedx_json(BOM).unwrap();
        ingest(&s, p.uuid, &bom).unwrap();
        let report2 = ingest(&s, p.uuid, &bom).unwrap();
        assert_eq!(report2.updated, 2);
        assert_eq!(report2.inserted, 0);
        // No duplicates inserted.
        assert_eq!(s.components_for(p.uuid).len(), 2);
    }

    #[test]
    fn ingest_unknown_project_fails() {
        let s = PortfolioStore::new();
        let bom = parse_cyclonedx_json(BOM).unwrap();
        assert!(ingest(&s, Uuid::new_v4(), &bom).is_err());
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Container-image scanner.
//!
//! Mirrors trivy's `pkg/scan/{image,local}` for cave-trivy MVP: given a
//! pre-resolved `ImageArtifact` (manifest digest + per-layer file blobs)
//! we extract OS family from /etc/os-release, parse the OS pkg db
//! (apk/dpkg/rpm), correlate against the offline vuln DB and produce a
//! `Report`. Live registry pulls are a scope cut — the orchestrator
//! consumes the materialised `ImageArtifact`.

use crate::error::TrivyResult;
use crate::models::{DetectedOs, OsFamily, Package, Report, ScanResult, Vulnerability};
use crate::pkg_os::{parse_apk_installed, parse_dpkg_status, parse_rpm_textdump, OsRelease};
use crate::vulndb::VulnDb;

#[derive(Debug, Clone, Default)]
pub struct ImageArtifact {
    pub name: String,
    pub digest: String,
    pub os_release: Option<String>,
    pub apk_db: Option<String>,
    pub dpkg_status: Option<String>,
    pub rpm_text: Option<String>,
    pub lockfiles: Vec<(String, String)>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ScanImageOpts {
    pub skip_lang_pkgs: bool,
    pub skip_os_pkgs: bool,
}

pub fn scan_image(art: &ImageArtifact, db: &VulnDb, opts: ScanImageOpts) -> TrivyResult<Report> {
    let mut report = Report::new(&art.name, "container_image");
    let os = art
        .os_release
        .as_deref()
        .and_then(OsRelease::parse);
    if let Some(o) = &os {
        report.os = Some(DetectedOs {
            family: o.family(),
            name: o.name.clone(),
        });
    }

    let os_pkgs = if opts.skip_os_pkgs {
        Vec::new()
    } else {
        extract_os_packages(art, os.as_ref())
    };
    let mut os_result = ScanResult {
        target: art.name.clone(),
        class: "os-pkgs".into(),
        ..Default::default()
    };
    correlate(db, &os_pkgs, &mut os_result.vulnerabilities);
    report.results.push(os_result);

    if !opts.skip_lang_pkgs {
        for (path, text) in &art.lockfiles {
            let base = path.rsplit('/').next().unwrap_or(path);
            let pkgs = crate::pkg_lang::parse_lockfile(base, text);
            let mut lr = ScanResult {
                target: path.clone(),
                class: "lang-pkgs".into(),
                ..Default::default()
            };
            correlate(db, &pkgs, &mut lr.vulnerabilities);
            report.results.push(lr);
        }
    }
    Ok(report)
}

pub fn extract_os_packages(art: &ImageArtifact, os: Option<&OsRelease>) -> Vec<Package> {
    let family = os.map(|o| o.family()).unwrap_or(OsFamily::Unknown);
    if let Some(text) = &art.apk_db {
        return parse_apk_installed(text);
    }
    if let Some(text) = &art.dpkg_status {
        return parse_dpkg_status(text);
    }
    if let Some(text) = &art.rpm_text {
        return parse_rpm_textdump(text, family);
    }
    Vec::new()
}

pub fn correlate(db: &VulnDb, pkgs: &[Package], out: &mut Vec<Vulnerability>) {
    for p in pkgs {
        for adv in db.match_pkg(&p.ecosystem, &p.name, &p.version) {
            out.push(Vulnerability {
                id: adv.id.clone(),
                pkg_name: p.name.clone(),
                installed_version: p.version.clone(),
                fixed_version: adv.fixed.clone(),
                severity: adv.severity,
                references: adv.references.clone(),
                title: Some(adv.title.clone()),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_alpine() -> ImageArtifact {
        ImageArtifact {
            name: "alpine:3.19".into(),
            digest: "sha256:aa".into(),
            os_release: Some("ID=alpine\nNAME=Alpine Linux\nVERSION_ID=3.19.1".into()),
            apk_db: Some("P:openssl\nV:3.0.0\n\nP:musl\nV:1.2.5\n".into()),
            ..Default::default()
        }
    }

    #[test]
    fn scan_alpine_finds_openssl_cve() {
        let r = scan_image(&fixture_alpine(), &VulnDb::cave_default(), ScanImageOpts::default()).unwrap();
        assert_eq!(r.results[0].class, "os-pkgs");
        assert!(r.results[0]
            .vulnerabilities
            .iter()
            .any(|v| v.id == "CVE-2026-0001"));
        assert_eq!(r.os.as_ref().unwrap().family, OsFamily::Alpine);
    }

    #[test]
    fn scan_lockfile_correlation() {
        let art = ImageArtifact {
            name: "node-app".into(),
            lockfiles: vec![(
                "/app/Cargo.lock".into(),
                "[[package]]\nname = \"openssl-sys\"\nversion = \"0.9.0\"\n".into(),
            )],
            ..Default::default()
        };
        let r = scan_image(&art, &VulnDb::cave_default(), ScanImageOpts::default()).unwrap();
        let langs: Vec<_> = r
            .results
            .iter()
            .filter(|s| s.class == "lang-pkgs")
            .collect();
        assert_eq!(langs.len(), 1);
        assert!(langs[0]
            .vulnerabilities
            .iter()
            .any(|v| v.id == "CVE-2026-0030"));
    }

    #[test]
    fn scan_debian_dpkg() {
        let art = ImageArtifact {
            name: "deb".into(),
            os_release: Some("ID=debian\nVERSION_ID=12".into()),
            dpkg_status: Some("Package: openssl\nVersion: 3.0.12\n".into()),
            ..Default::default()
        };
        let r = scan_image(&art, &VulnDb::cave_default(), ScanImageOpts::default()).unwrap();
        assert!(r.results[0]
            .vulnerabilities
            .iter()
            .any(|v| v.id == "CVE-2026-0003"));
    }

    #[test]
    fn scan_skip_os() {
        let opts = ScanImageOpts {
            skip_os_pkgs: true,
            skip_lang_pkgs: false,
        };
        let r = scan_image(&fixture_alpine(), &VulnDb::cave_default(), opts).unwrap();
        assert!(r.results[0].vulnerabilities.is_empty());
    }

    #[test]
    fn scan_skip_lang() {
        let mut art = fixture_alpine();
        art.lockfiles.push(("Cargo.lock".into(), "[[package]]\nname=\"x\"\nversion=\"1\"\n".into()));
        let opts = ScanImageOpts {
            skip_lang_pkgs: true,
            skip_os_pkgs: false,
        };
        let r = scan_image(&art, &VulnDb::cave_default(), opts).unwrap();
        assert_eq!(r.results.len(), 1);
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! OS package detection.
//!
//! Mirrors trivy's `pkg/fanal/analyzer/pkg/{apk,dpkg,rpm,etc}` for the
//! subset cave-trivy MVP supports: Alpine `lib/apk/db/installed`,
//! Debian/Ubuntu `var/lib/dpkg/status`, RPM-family
//! `var/lib/rpm/Packages` (text headers — the binary BDB/SQLite reader is
//! a scope cut), plus a `/etc/os-release` parser used by all OS scanners
//! to identify the family.

use crate::models::{OsFamily, Package};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OsRelease {
    pub id: String,
    pub name: String,
    pub version_id: String,
}

impl OsRelease {
    pub fn parse(text: &str) -> Option<Self> {
        let mut id = None;
        let mut name = None;
        let mut version = None;
        for line in text.lines() {
            let l = line.trim();
            if l.is_empty() || l.starts_with('#') {
                continue;
            }
            let (k, v) = match l.split_once('=') {
                Some(t) => t,
                None => continue,
            };
            let v = v.trim().trim_matches('"').to_string();
            match k {
                "ID" => id = Some(v),
                "NAME" => name = Some(v),
                "VERSION_ID" => version = Some(v),
                _ => {}
            }
        }
        let id = id?;
        Some(Self {
            id: id.clone(),
            name: name.unwrap_or(id.clone()),
            version_id: version.unwrap_or_default(),
        })
    }

    pub fn family(&self) -> OsFamily {
        OsFamily::from_id(&self.id)
    }
}

/// Alpine `installed` DB text format. Each package is a block of `K:V`
/// lines separated by a blank line. `P:` = package, `V:` = version.
pub fn parse_apk_installed(text: &str) -> Vec<Package> {
    let mut out = Vec::new();
    let mut name = String::new();
    let mut version = String::new();
    for line in text.lines() {
        if line.is_empty() {
            if !name.is_empty() && !version.is_empty() {
                out.push(Package::new(&name, &version, "alpine"));
            }
            name.clear();
            version.clear();
            continue;
        }
        let (k, v) = match line.split_once(':') {
            Some(t) => t,
            None => continue,
        };
        match k {
            "P" => name = v.to_string(),
            "V" => version = v.to_string(),
            _ => {}
        }
    }
    if !name.is_empty() && !version.is_empty() {
        out.push(Package::new(&name, &version, "alpine"));
    }
    out
}

/// Debian/Ubuntu `var/lib/dpkg/status` text format. Each package is a
/// block of `Key: Value` lines separated by a blank line.
pub fn parse_dpkg_status(text: &str) -> Vec<Package> {
    let mut out = Vec::new();
    let mut name = String::new();
    let mut version = String::new();
    let mut source = String::new();
    let mut ecosystem = "debian";
    for line in text.lines() {
        if line.is_empty() {
            if !name.is_empty() && !version.is_empty() {
                let mut p = Package::new(&name, &version, ecosystem);
                if !source.is_empty() {
                    p.source = Some(source.clone());
                }
                out.push(p);
            }
            name.clear();
            version.clear();
            source.clear();
            continue;
        }
        let (k, v) = match line.split_once(": ") {
            Some(t) => t,
            None => continue,
        };
        match k {
            "Package" => name = v.trim().to_string(),
            "Version" => version = v.trim().to_string(),
            "Source" => source = v.trim().to_string(),
            "Origin" if v.to_ascii_lowercase().contains("ubuntu") => ecosystem = "ubuntu",
            _ => {}
        }
    }
    if !name.is_empty() && !version.is_empty() {
        let mut p = Package::new(&name, &version, ecosystem);
        if !source.is_empty() {
            p.source = Some(source);
        }
        out.push(p);
    }
    out
}

/// Trivial RPM header text format: one `Name|Version|Release|Arch` per
/// line. cave-trivy's offline-friendly format — the upstream binary BDB
/// and SQLite Packages readers are a scope cut.
pub fn parse_rpm_textdump(text: &str, family: OsFamily) -> Vec<Package> {
    let eco = match family {
        OsFamily::Rhel | OsFamily::Centos | OsFamily::Rocky | OsFamily::Alma => "rhel",
        OsFamily::Amazon => "amazon",
        OsFamily::Oracle => "oracle",
        OsFamily::Photon => "photon",
        OsFamily::Mariner => "mariner",
        OsFamily::Suse | OsFamily::OpenSuse => "suse",
        _ => "rpm",
    };
    let mut out = Vec::new();
    for line in text.lines() {
        let parts: Vec<&str> = line.split('|').collect();
        if parts.len() < 2 {
            continue;
        }
        let name = parts[0].trim();
        let version = parts[1].trim();
        if name.is_empty() || version.is_empty() {
            continue;
        }
        let mut p = Package::new(name, version, eco);
        if parts.len() >= 3 {
            p.release = Some(parts[2].trim().to_string());
        }
        out.push(p);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_os_release_alpine() {
        let r = OsRelease::parse(
            r#"ID=alpine
NAME="Alpine Linux"
VERSION_ID=3.19.1
"#,
        )
        .unwrap();
        assert_eq!(r.family(), OsFamily::Alpine);
        assert_eq!(r.version_id, "3.19.1");
    }

    #[test]
    fn parse_os_release_debian() {
        let r = OsRelease::parse("ID=debian\nVERSION_ID=12\n").unwrap();
        assert_eq!(r.family(), OsFamily::Debian);
    }

    #[test]
    fn parse_os_release_none() {
        assert!(OsRelease::parse("# blank\n").is_none());
    }

    #[test]
    fn parse_apk_two_packages() {
        let text = "P:openssl\nV:3.0.0\n\nP:musl\nV:1.2.5\n";
        let pkgs = parse_apk_installed(text);
        assert_eq!(pkgs.len(), 2);
        assert_eq!(pkgs[0].name, "openssl");
        assert_eq!(pkgs[1].version, "1.2.5");
    }

    #[test]
    fn parse_apk_trailing_block() {
        let text = "P:curl\nV:8.5.0";
        let pkgs = parse_apk_installed(text);
        assert_eq!(pkgs.len(), 1);
        assert_eq!(pkgs[0].name, "curl");
    }

    #[test]
    fn parse_dpkg_source_origin() {
        let text = "Package: openssl\nVersion: 3.0.13-1\nSource: openssl\nOrigin: Ubuntu\n\nPackage: curl\nVersion: 8.5.0-1\n";
        let pkgs = parse_dpkg_status(text);
        assert_eq!(pkgs.len(), 2);
        assert_eq!(pkgs[0].ecosystem, "ubuntu");
        assert_eq!(pkgs[0].source.as_deref(), Some("openssl"));
    }

    #[test]
    fn parse_rpm_textdump_minimal() {
        let text = "kernel|5.14.0|499.el9|x86_64\nopenssl|3.0.7|18.el9\n";
        let pkgs = parse_rpm_textdump(text, OsFamily::Rhel);
        assert_eq!(pkgs.len(), 2);
        assert_eq!(pkgs[0].ecosystem, "rhel");
        assert_eq!(pkgs[1].release.as_deref(), Some("18.el9"));
    }

    #[test]
    fn parse_rpm_amazon() {
        let pkgs = parse_rpm_textdump("kernel|5.10|amzn", OsFamily::Amazon);
        assert_eq!(pkgs[0].ecosystem, "amazon");
    }
}

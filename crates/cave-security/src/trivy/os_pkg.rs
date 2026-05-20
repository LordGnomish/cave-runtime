// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! OS package extraction — Alpine APK, Debian dpkg, RHEL/CentOS RPM.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackageManager {
    Apk,
    Dpkg,
    Rpm,
}

impl std::fmt::Display for PackageManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PackageManager::Apk => write!(f, "apk"),
            PackageManager::Dpkg => write!(f, "dpkg"),
            PackageManager::Rpm => write!(f, "rpm"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OsPackage {
    pub name: String,
    pub version: String,
    pub arch: Option<String>,
    pub source_name: Option<String>,
    pub source_version: Option<String>,
    pub licenses: Vec<String>,
    pub maintainer: Option<String>,
    pub package_manager: PackageManager,
}

/// OS type detected from the filesystem.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OsInfo {
    pub family: OsFamily,
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OsFamily {
    Alpine,
    Debian,
    Ubuntu,
    Rhel,
    Centos,
    Fedora,
    AmazonLinux,
    Unknown,
}

// ---------------------------------------------------------------------------
// OS detection
// ---------------------------------------------------------------------------

/// Detect OS from `/etc/os-release` content.
pub fn detect_os(os_release: &str) -> OsInfo {
    let id = extract_value(os_release, "ID")
        .unwrap_or_default()
        .to_lowercase();
    let name = extract_value(os_release, "PRETTY_NAME")
        .or_else(|| extract_value(os_release, "NAME"))
        .unwrap_or_default();
    let version = extract_value(os_release, "VERSION_ID").unwrap_or_default();

    let family = match id.as_str() {
        "alpine" => OsFamily::Alpine,
        "debian" => OsFamily::Debian,
        "ubuntu" => OsFamily::Ubuntu,
        "rhel" | "redhat" => OsFamily::Rhel,
        "centos" => OsFamily::Centos,
        "fedora" => OsFamily::Fedora,
        "amzn" | "amazon" => OsFamily::AmazonLinux,
        _ => OsFamily::Unknown,
    };

    OsInfo {
        family,
        name: name.trim_matches('"').to_string(),
        version: version.trim_matches('"').to_string(),
    }
}

fn extract_value<'a>(content: &'a str, key: &str) -> Option<String> {
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix(key) {
            if let Some(rest) = rest.strip_prefix('=') {
                return Some(rest.trim().trim_matches('"').to_string());
            }
        }
    }
    None
}

/// Detect Alpine from `/etc/alpine-release`.
pub fn detect_alpine_version(alpine_release: &str) -> OsInfo {
    OsInfo {
        family: OsFamily::Alpine,
        name: "Alpine Linux".into(),
        version: alpine_release.trim().to_string(),
    }
}

// ---------------------------------------------------------------------------
// Alpine APK parser — /lib/apk/db/installed
// ---------------------------------------------------------------------------

/// Parse Alpine APK installed database.
pub fn parse_apk_installed(content: &str) -> Vec<OsPackage> {
    let mut packages = Vec::new();
    let mut current: Option<OsPackage> = None;

    for line in content.lines() {
        if line.is_empty() {
            if let Some(pkg) = current.take() {
                packages.push(pkg);
            }
            continue;
        }
        if let Some((tag, value)) = line.split_once(':') {
            let value = value.trim();
            match tag {
                "P" => {
                    current = Some(OsPackage {
                        name: value.to_string(),
                        version: String::new(),
                        arch: None,
                        source_name: None,
                        source_version: None,
                        licenses: vec![],
                        maintainer: None,
                        package_manager: PackageManager::Apk,
                    });
                }
                "V" => {
                    if let Some(ref mut pkg) = current {
                        pkg.version = value.to_string();
                    }
                }
                "A" => {
                    if let Some(ref mut pkg) = current {
                        pkg.arch = Some(value.to_string());
                    }
                }
                "L" => {
                    if let Some(ref mut pkg) = current {
                        pkg.licenses = value.split(' ').map(str::to_string).collect();
                    }
                }
                "m" => {
                    if let Some(ref mut pkg) = current {
                        pkg.maintainer = Some(value.to_string());
                    }
                }
                "o" => {
                    if let Some(ref mut pkg) = current {
                        pkg.source_name = Some(value.to_string());
                    }
                }
                _ => {}
            }
        }
    }
    if let Some(pkg) = current {
        packages.push(pkg);
    }
    packages
}

// ---------------------------------------------------------------------------
// Debian dpkg parser — /var/lib/dpkg/status
// ---------------------------------------------------------------------------

/// Parse Debian dpkg status file.
pub fn parse_dpkg_status(content: &str) -> Vec<OsPackage> {
    let mut packages = Vec::new();
    let mut current: Option<OsPackage> = None;

    for line in content.lines() {
        if line.is_empty() {
            if let Some(pkg) = current.take() {
                if !pkg.name.is_empty() && !pkg.version.is_empty() {
                    packages.push(pkg);
                }
            }
            continue;
        }
        if let Some((key, value)) = line.split_once(": ") {
            let value = value.trim();
            match key {
                "Package" => {
                    current = Some(OsPackage {
                        name: value.to_string(),
                        version: String::new(),
                        arch: None,
                        source_name: None,
                        source_version: None,
                        licenses: vec![],
                        maintainer: None,
                        package_manager: PackageManager::Dpkg,
                    });
                }
                "Version" => {
                    if let Some(ref mut pkg) = current {
                        pkg.version = value.to_string();
                    }
                }
                "Architecture" => {
                    if let Some(ref mut pkg) = current {
                        pkg.arch = Some(value.to_string());
                    }
                }
                "Maintainer" => {
                    if let Some(ref mut pkg) = current {
                        pkg.maintainer = Some(value.to_string());
                    }
                }
                "Source" => {
                    if let Some(ref mut pkg) = current {
                        // Source: packagename (version)
                        if let Some((src, src_ver)) = value.split_once(' ') {
                            pkg.source_name = Some(src.to_string());
                            pkg.source_version =
                                Some(src_ver.trim_matches(|c| c == '(' || c == ')').to_string());
                        } else {
                            pkg.source_name = Some(value.to_string());
                        }
                    }
                }
                _ => {}
            }
        }
    }
    if let Some(pkg) = current {
        if !pkg.name.is_empty() && !pkg.version.is_empty() {
            packages.push(pkg);
        }
    }
    packages
}

// ---------------------------------------------------------------------------
// RPM parser — /var/lib/rpm/Packages (simplified text representation)
// ---------------------------------------------------------------------------

/// Parse RPM package list from `rpm -qa --queryformat` output.
/// Expected format: NAME|VERSION-RELEASE|ARCH|LICENSE per line.
pub fn parse_rpm_query_output(content: &str) -> Vec<OsPackage> {
    let mut packages = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split('|').collect();
        if parts.len() >= 2 {
            packages.push(OsPackage {
                name: parts[0].to_string(),
                version: parts[1].to_string(),
                arch: parts.get(2).map(|s| s.to_string()),
                source_name: None,
                source_version: None,
                licenses: parts
                    .get(3)
                    .map(|s| s.split(" and ").map(str::to_string).collect())
                    .unwrap_or_default(),
                maintainer: None,
                package_manager: PackageManager::Rpm,
            });
        }
    }
    packages
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const APK_SAMPLE: &str = "\
P:musl
V:1.2.4-r2
A:x86_64
L:MIT
m:Timo Teräs <timo.teras@iki.fi>

P:busybox
V:1.36.1-r5
A:x86_64
L:GPL-2.0-only

";

    const DPKG_SAMPLE: &str = "\
Package: libc6
Version: 2.36-9+deb12u4
Architecture: amd64
Maintainer: GNU Libc Maintainers <debian-glibc@lists.debian.org>

Package: openssl
Version: 3.0.11-1~deb12u2
Architecture: amd64

";

    #[test]
    fn parse_apk() {
        let pkgs = parse_apk_installed(APK_SAMPLE);
        assert_eq!(pkgs.len(), 2);
        assert_eq!(pkgs[0].name, "musl");
        assert_eq!(pkgs[0].version, "1.2.4-r2");
        assert!(pkgs[0].licenses.contains(&"MIT".to_string()));
    }

    #[test]
    fn parse_dpkg() {
        let pkgs = parse_dpkg_status(DPKG_SAMPLE);
        assert_eq!(pkgs.len(), 2);
        assert_eq!(pkgs[1].name, "openssl");
        assert_eq!(pkgs[1].version, "3.0.11-1~deb12u2");
    }

    #[test]
    fn parse_rpm() {
        let content = "bash|5.2.15-3.fc38|x86_64|GPL-3.0\nopenssl|3.1.1-2.fc38|x86_64|Apache-2.0\n";
        let pkgs = parse_rpm_query_output(content);
        assert_eq!(pkgs.len(), 2);
        assert_eq!(pkgs[0].name, "bash");
    }

    #[test]
    fn detect_alpine_os() {
        let os_release = "ID=alpine\nVERSION_ID=3.18\nPRETTY_NAME=\"Alpine Linux v3.18\"\n";
        let os = detect_os(os_release);
        assert_eq!(os.family, OsFamily::Alpine);
        assert_eq!(os.version, "3.18");
    }

    #[test]
    fn detect_debian_os() {
        let os_release =
            "ID=debian\nVERSION_ID=\"12\"\nPRETTY_NAME=\"Debian GNU/Linux 12 (bookworm)\"\n";
        let os = detect_os(os_release);
        assert_eq!(os.family, OsFamily::Debian);
        assert_eq!(os.version, "12");
    }
}

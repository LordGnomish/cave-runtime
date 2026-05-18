// SPDX-License-Identifier: AGPL-3.0-or-later
//! Vulnerability database — NVD-style CVE records, in-memory store.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Severity
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Hash, Default)]
#[serde(rename_all = "UPPERCASE")]
pub enum Severity {
    #[default]
    Unknown,
    Low,
    Medium,
    High,
    Critical,
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Severity::Unknown => write!(f, "UNKNOWN"),
            Severity::Low => write!(f, "LOW"),
            Severity::Medium => write!(f, "MEDIUM"),
            Severity::High => write!(f, "HIGH"),
            Severity::Critical => write!(f, "CRITICAL"),
        }
    }
}

impl Severity {
    pub fn from_cvss_score(score: f32) -> Self {
        match score as u32 {
            0 => Severity::Unknown,
            1..=3 => Severity::Low,
            4..=6 => Severity::Medium,
            7..=8 => Severity::High,
            _ => Severity::Critical,
        }
    }
}

// ---------------------------------------------------------------------------
// CVSS
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cvss {
    pub v2_score: Option<f32>,
    pub v3_score: Option<f32>,
    pub v3_vector: Option<String>,
}

// ---------------------------------------------------------------------------
// CVE Record
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VulnRecord {
    pub cve_id: String,
    pub title: String,
    pub description: String,
    pub severity: Severity,
    pub cvss: Option<Cvss>,
    pub cwe: Vec<String>,
    pub references: Vec<String>,
    pub published: DateTime<Utc>,
    pub last_modified: DateTime<Utc>,
    /// Affected packages: ecosystem → package name → affected version ranges
    pub affected: Vec<AffectedPackage>,
    pub fixed_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AffectedPackage {
    pub ecosystem: String,
    pub package_name: String,
    pub vulnerable_versions: Vec<String>, // e.g., "< 1.2.3", ">= 2.0, < 2.1.5"
    pub patched_version: Option<String>,
}

// ---------------------------------------------------------------------------
// VulnDb
// ---------------------------------------------------------------------------

pub struct VulnDb {
    /// CVE ID → record
    by_cve: HashMap<String, VulnRecord>,
    /// "ecosystem/package" → list of CVE IDs
    by_package: HashMap<String, Vec<String>>,
    pub last_updated: Option<DateTime<Utc>>,
}

impl Default for VulnDb {
    fn default() -> Self {
        let mut db = VulnDb {
            by_cve: HashMap::new(),
            by_package: HashMap::new(),
            last_updated: None,
        };
        db.load_builtin();
        db
    }
}

impl VulnDb {
    pub fn get(&self, cve_id: &str) -> Option<&VulnRecord> {
        self.by_cve.get(cve_id)
    }

    pub fn lookup_package(&self, ecosystem: &str, pkg: &str) -> Vec<&VulnRecord> {
        let key = format!("{ecosystem}/{pkg}");
        self.by_package
            .get(&key)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| self.by_cve.get(id))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn ingest(&mut self, records: Vec<VulnRecord>) {
        for r in records {
            let cve = r.cve_id.clone();
            for aff in &r.affected {
                let key = format!("{}/{}", aff.ecosystem, aff.package_name);
                self.by_package.entry(key).or_default().push(cve.clone());
            }
            self.by_cve.insert(cve, r);
        }
        self.last_updated = Some(Utc::now());
    }

    pub fn all_cves(&self) -> impl Iterator<Item = &VulnRecord> {
        self.by_cve.values()
    }

    pub fn len(&self) -> usize {
        self.by_cve.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_cve.is_empty()
    }

    fn load_builtin(&mut self) {
        self.ingest(builtin_records());
    }
}

// ---------------------------------------------------------------------------
// Builtin vulnerability records (representative sample)
// ---------------------------------------------------------------------------

pub fn builtin_records() -> Vec<VulnRecord> {
    use chrono::TimeZone;

    vec![
        VulnRecord {
            cve_id: "CVE-2021-44228".into(),
            title: "Log4Shell: Remote code execution in log4j".into(),
            description: "Apache Log4j2 <= 2.14.1 JNDI features allow remote code execution.".into(),
            severity: Severity::Critical,
            cvss: Some(Cvss {
                v2_score: None,
                v3_score: Some(10.0),
                v3_vector: Some("CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:C/C:H/I:H/A:H".into()),
            }),
            cwe: vec!["CWE-917".into(), "CWE-502".into()],
            references: vec!["https://nvd.nist.gov/vuln/detail/CVE-2021-44228".into()],
            published: Utc.with_ymd_and_hms(2021, 12, 10, 0, 0, 0).unwrap(),
            last_modified: Utc.with_ymd_and_hms(2022, 7, 1, 0, 0, 0).unwrap(),
            affected: vec![AffectedPackage {
                ecosystem: "maven".into(),
                package_name: "org.apache.logging.log4j:log4j-core".into(),
                vulnerable_versions: vec!["< 2.15.0".into()],
                patched_version: Some("2.15.0".into()),
            }],
            fixed_version: Some("2.15.0".into()),
        },
        VulnRecord {
            cve_id: "CVE-2022-22965".into(),
            title: "Spring4Shell: RCE in Spring Framework".into(),
            description: "Spring Framework RCE with JDK 9+ via DataBinder.".into(),
            severity: Severity::Critical,
            cvss: Some(Cvss {
                v2_score: None,
                v3_score: Some(9.8),
                v3_vector: Some("CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H".into()),
            }),
            cwe: vec!["CWE-94".into()],
            references: vec!["https://nvd.nist.gov/vuln/detail/CVE-2022-22965".into()],
            published: Utc.with_ymd_and_hms(2022, 3, 31, 0, 0, 0).unwrap(),
            last_modified: Utc.with_ymd_and_hms(2022, 4, 15, 0, 0, 0).unwrap(),
            affected: vec![AffectedPackage {
                ecosystem: "maven".into(),
                package_name: "org.springframework:spring-webmvc".into(),
                vulnerable_versions: vec!["< 5.3.18".into(), "< 5.2.20".into()],
                patched_version: Some("5.3.18".into()),
            }],
            fixed_version: Some("5.3.18".into()),
        },
        VulnRecord {
            cve_id: "CVE-2023-44487".into(),
            title: "HTTP/2 Rapid Reset Attack".into(),
            description: "HTTP/2 rapid reset can cause DoS via stream cancellation loop.".into(),
            severity: Severity::High,
            cvss: Some(Cvss {
                v2_score: None,
                v3_score: Some(7.5),
                v3_vector: Some("CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:N/I:N/A:H".into()),
            }),
            cwe: vec!["CWE-400".into()],
            references: vec!["https://nvd.nist.gov/vuln/detail/CVE-2023-44487".into()],
            published: Utc.with_ymd_and_hms(2023, 10, 10, 0, 0, 0).unwrap(),
            last_modified: Utc.with_ymd_and_hms(2023, 10, 25, 0, 0, 0).unwrap(),
            affected: vec![
                AffectedPackage {
                    ecosystem: "go".into(),
                    package_name: "golang.org/x/net".into(),
                    vulnerable_versions: vec!["< 0.17.0".into()],
                    patched_version: Some("0.17.0".into()),
                },
                AffectedPackage {
                    ecosystem: "npm".into(),
                    package_name: "node".into(),
                    vulnerable_versions: vec!["< 18.18.2".into(), "< 20.8.1".into()],
                    patched_version: Some("18.18.2".into()),
                },
            ],
            fixed_version: Some("0.17.0".into()),
        },
        VulnRecord {
            cve_id: "CVE-2021-41773".into(),
            title: "Apache HTTP Server path traversal".into(),
            description: "Path traversal and RCE in Apache HTTP Server 2.4.49.".into(),
            severity: Severity::Critical,
            cvss: Some(Cvss {
                v2_score: Some(7.5),
                v3_score: Some(9.8),
                v3_vector: None,
            }),
            cwe: vec!["CWE-22".into()],
            references: vec!["https://nvd.nist.gov/vuln/detail/CVE-2021-41773".into()],
            published: Utc.with_ymd_and_hms(2021, 10, 4, 0, 0, 0).unwrap(),
            last_modified: Utc.with_ymd_and_hms(2021, 10, 14, 0, 0, 0).unwrap(),
            affected: vec![AffectedPackage {
                ecosystem: "os/alpine".into(),
                package_name: "apache2".into(),
                vulnerable_versions: vec!["= 2.4.49".into()],
                patched_version: Some("2.4.50".into()),
            }],
            fixed_version: Some("2.4.50".into()),
        },
        VulnRecord {
            cve_id: "CVE-2023-0464".into(),
            title: "OpenSSL denial of service via X.509 policy constraints".into(),
            description: "A security vulnerability in OpenSSL's X.509 cert verification.".into(),
            severity: Severity::High,
            cvss: Some(Cvss {
                v2_score: None,
                v3_score: Some(7.5),
                v3_vector: None,
            }),
            cwe: vec!["CWE-295".into()],
            references: vec!["https://nvd.nist.gov/vuln/detail/CVE-2023-0464".into()],
            published: Utc.with_ymd_and_hms(2023, 3, 22, 0, 0, 0).unwrap(),
            last_modified: Utc.with_ymd_and_hms(2023, 4, 5, 0, 0, 0).unwrap(),
            affected: vec![
                AffectedPackage {
                    ecosystem: "os/alpine".into(),
                    package_name: "openssl".into(),
                    vulnerable_versions: vec!["< 3.0.9".into()],
                    patched_version: Some("3.0.9".into()),
                },
                AffectedPackage {
                    ecosystem: "os/debian".into(),
                    package_name: "openssl".into(),
                    vulnerable_versions: vec!["< 3.0.9-1".into()],
                    patched_version: Some("3.0.9-1".into()),
                },
            ],
            fixed_version: Some("3.0.9".into()),
        },
        VulnRecord {
            cve_id: "CVE-2023-38408".into(),
            title: "OpenSSH ssh-agent remote code execution".into(),
            description: "Remote code execution via ssh-agent forwarding in OpenSSH.".into(),
            severity: Severity::Critical,
            cvss: Some(Cvss {
                v2_score: None,
                v3_score: Some(9.8),
                v3_vector: None,
            }),
            cwe: vec!["CWE-428".into()],
            references: vec!["https://nvd.nist.gov/vuln/detail/CVE-2023-38408".into()],
            published: Utc.with_ymd_and_hms(2023, 7, 19, 0, 0, 0).unwrap(),
            last_modified: Utc.with_ymd_and_hms(2023, 7, 31, 0, 0, 0).unwrap(),
            affected: vec![
                AffectedPackage {
                    ecosystem: "os/alpine".into(),
                    package_name: "openssh".into(),
                    vulnerable_versions: vec!["< 9.3p2".into()],
                    patched_version: Some("9.3p2".into()),
                },
                AffectedPackage {
                    ecosystem: "os/debian".into(),
                    package_name: "openssh".into(),
                    vulnerable_versions: vec!["< 1:9.2p1-2+deb12u3".into()],
                    patched_version: Some("1:9.2p1-2+deb12u3".into()),
                },
            ],
            fixed_version: Some("9.3p2".into()),
        },
        VulnRecord {
            cve_id: "CVE-2022-3786".into(),
            title: "OpenSSL X.509 Email Address Buffer Overflow".into(),
            description: "Buffer overflow in X.509 certificate verification in OpenSSL 3.0.x.".into(),
            severity: Severity::High,
            cvss: Some(Cvss {
                v2_score: None,
                v3_score: Some(7.5),
                v3_vector: None,
            }),
            cwe: vec!["CWE-120".into()],
            references: vec!["https://nvd.nist.gov/vuln/detail/CVE-2022-3786".into()],
            published: Utc.with_ymd_and_hms(2022, 11, 1, 0, 0, 0).unwrap(),
            last_modified: Utc.with_ymd_and_hms(2022, 11, 15, 0, 0, 0).unwrap(),
            affected: vec![AffectedPackage {
                ecosystem: "go".into(),
                package_name: "stdlib".into(),
                vulnerable_versions: vec!["< 1.19.3".into(), "< 1.18.8".into()],
                patched_version: Some("1.19.3".into()),
            }],
            fixed_version: Some("1.19.3".into()),
        },
    ]
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_db_has_records() {
        let db = VulnDb::default();
        assert!(!db.is_empty());
    }

    #[test]
    fn get_by_cve_id() {
        let db = VulnDb::default();
        let rec = db.get("CVE-2021-44228").unwrap();
        assert!(rec.title.contains("Log4Shell"));
        assert_eq!(rec.severity, Severity::Critical);
    }

    #[test]
    fn lookup_by_package() {
        let db = VulnDb::default();
        let vulns = db.lookup_package("maven", "org.apache.logging.log4j:log4j-core");
        assert!(!vulns.is_empty());
        assert!(vulns.iter().any(|v| v.cve_id == "CVE-2021-44228"));
    }

    #[test]
    fn ingest_custom_record() {
        let mut db = VulnDb::default();
        let before = db.len();
        db.ingest(vec![VulnRecord {
            cve_id: "CVE-2099-99999".into(),
            title: "Future bug".into(),
            description: "TBD".into(),
            severity: Severity::Medium,
            cvss: None,
            cwe: vec![],
            references: vec![],
            published: Utc::now(),
            last_modified: Utc::now(),
            affected: vec![],
            fixed_version: None,
        }]);
        assert_eq!(db.len(), before + 1);
    }

    #[test]
    fn severity_from_cvss() {
        assert_eq!(Severity::from_cvss_score(9.5), Severity::Critical);
        assert_eq!(Severity::from_cvss_score(7.0), Severity::High);
        assert_eq!(Severity::from_cvss_score(5.0), Severity::Medium);
        assert_eq!(Severity::from_cvss_score(2.5), Severity::Low);
    }
}

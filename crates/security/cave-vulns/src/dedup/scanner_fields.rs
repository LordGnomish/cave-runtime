// SPDX-License-Identifier: AGPL-3.0-or-later
//! Per-scanner hash-field tuples — DefectDojo's HASHCODE_FIELDS_PER_SCANNER.
//!
//! Source: DefectDojo/django-DefectDojo@6eab8738
//!         dojo/settings/settings.dist.py:978-1135
//!
//! Ported subset: all scanners we have a parser for, plus a curated
//! list of high-traffic SAST/DAST/SCA tools. Unknown scanners fall
//! through to the legacy 5-field set in [super::hash_code_for].

use crate::finding::Finding;

/// One field that participates in a scanner's hash_code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HashField {
    Title,
    Severity,
    Cwe,
    Cve,
    Line,
    FilePath,
    Description,
    ComponentName,
    ComponentVersion,
    VulnIdFromTool,
    VulnerabilityIds,
}

impl HashField {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Title => "title",
            Self::Severity => "severity",
            Self::Cwe => "cwe",
            Self::Cve => "cve",
            Self::Line => "line",
            Self::FilePath => "file_path",
            Self::Description => "description",
            Self::ComponentName => "component_name",
            Self::ComponentVersion => "component_version",
            Self::VulnIdFromTool => "vuln_id_from_tool",
            Self::VulnerabilityIds => "vulnerability_ids",
        }
    }

    pub fn value(&self, f: &Finding) -> String {
        match self {
            Self::Title => f.title.clone(),
            Self::Severity => f.severity.as_str().to_string(),
            Self::Cwe => f.cwe.map(|c| c.to_string()).unwrap_or_default(),
            Self::Cve => f.cve.clone().unwrap_or_default(),
            Self::Line => f.line.map(|l| l.to_string()).unwrap_or_default(),
            Self::FilePath => f.file_path.clone().unwrap_or_default(),
            Self::Description => f.description.clone(),
            Self::ComponentName => f.component_name.clone().unwrap_or_default(),
            Self::ComponentVersion => f.component_version.clone().unwrap_or_default(),
            Self::VulnIdFromTool => f.vuln_id_from_tool.clone().unwrap_or_default(),
            Self::VulnerabilityIds => f.vulnerability_ids.join(","),
        }
    }
}

/// Per-scanner field tuple table — straight transliteration of
/// HASHCODE_FIELDS_PER_SCANNER for the parsers we ship.
///
/// Source: DefectDojo/django-DefectDojo@6eab8738 dojo/settings/settings.dist.py
///         lines 978-1135 (subset).
pub static HASHCODE_FIELDS_PER_SCANNER: &[(&str, &[HashField])] = &[
    (
        "Bandit Scan",
        &[
            HashField::FilePath,
            HashField::Line,
            HashField::VulnIdFromTool,
        ],
    ),
    (
        "ZAP Scan",
        &[HashField::Title, HashField::Cwe, HashField::Severity],
    ),
    (
        "Trivy Scan",
        &[
            HashField::Title,
            HashField::Severity,
            HashField::VulnerabilityIds,
            HashField::Cwe,
            HashField::Description,
        ],
    ),
    (
        "Semgrep JSON Report",
        &[
            HashField::Title,
            HashField::Cwe,
            HashField::Line,
            HashField::FilePath,
            HashField::Description,
        ],
    ),
    (
        "SARIF",
        &[
            HashField::Title,
            HashField::Cwe,
            HashField::Line,
            HashField::FilePath,
            HashField::Description,
        ],
    ),
    (
        "Snyk Scan",
        &[
            HashField::VulnIdFromTool,
            HashField::FilePath,
            HashField::ComponentName,
            HashField::ComponentVersion,
        ],
    ),
    (
        "Nuclei Scan",
        &[
            HashField::Title,
            HashField::Severity,
            HashField::VulnIdFromTool,
        ],
    ),
    (
        "Anchore Engine Scan",
        &[
            HashField::Title,
            HashField::Severity,
            HashField::ComponentName,
            HashField::ComponentVersion,
            HashField::FilePath,
        ],
    ),
    (
        "Anchore Grype",
        &[
            HashField::Title,
            HashField::Severity,
            HashField::ComponentName,
            HashField::ComponentVersion,
        ],
    ),
    (
        "Aqua Scan",
        &[
            HashField::Severity,
            HashField::VulnerabilityIds,
            HashField::ComponentName,
            HashField::ComponentVersion,
        ],
    ),
    (
        "Burp Scan",
        &[
            HashField::Title,
            HashField::Severity,
            HashField::VulnIdFromTool,
        ],
    ),
    (
        "CargoAudit Scan",
        &[
            HashField::VulnerabilityIds,
            HashField::Severity,
            HashField::ComponentName,
            HashField::ComponentVersion,
            HashField::VulnIdFromTool,
        ],
    ),
    (
        "Checkmarx Scan",
        &[HashField::Cwe, HashField::Severity, HashField::FilePath],
    ),
    (
        "Cloudsploit Scan",
        &[HashField::Title, HashField::Description],
    ),
    (
        "SonarQube Scan",
        &[HashField::Cwe, HashField::Severity, HashField::FilePath],
    ),
    (
        "Dependency Check Scan",
        &[HashField::Title, HashField::Cwe, HashField::FilePath],
    ),
    (
        "NPM Audit Scan",
        &[
            HashField::Title,
            HashField::Severity,
            HashField::FilePath,
            HashField::VulnerabilityIds,
            HashField::Cwe,
        ],
    ),
    (
        "Yarn Audit Scan",
        &[
            HashField::Title,
            HashField::Severity,
            HashField::FilePath,
            HashField::VulnerabilityIds,
            HashField::Cwe,
        ],
    ),
    (
        "GitLab Dependency Scanning Report",
        &[
            HashField::Title,
            HashField::VulnerabilityIds,
            HashField::FilePath,
            HashField::ComponentName,
            HashField::ComponentVersion,
        ],
    ),
    (
        "Github SAST Scan",
        &[
            HashField::VulnIdFromTool,
            HashField::Severity,
            HashField::FilePath,
            HashField::Line,
        ],
    ),
    (
        "TFSec Scan",
        &[
            HashField::Severity,
            HashField::VulnIdFromTool,
            HashField::FilePath,
            HashField::Line,
        ],
    ),
    (
        "Tenable Scan",
        &[
            HashField::Title,
            HashField::Severity,
            HashField::VulnerabilityIds,
            HashField::Cwe,
            HashField::Description,
        ],
    ),
];

/// Lookup the field tuple for `scanner`. Falls back to the legacy
/// 5-field set when unknown — matches DefectDojo's default.
pub fn fields_for_scanner(scanner: &str) -> &'static [HashField] {
    for (name, fields) in HASHCODE_FIELDS_PER_SCANNER {
        if *name == scanner {
            return fields;
        }
    }
    const LEGACY: &[HashField] = &[
        HashField::Title,
        HashField::Cwe,
        HashField::Line,
        HashField::FilePath,
        HashField::Description,
    ];
    LEGACY
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_scanner_returns_explicit_fields() {
        let fields = fields_for_scanner("Bandit Scan");
        assert_eq!(
            fields,
            &[
                HashField::FilePath,
                HashField::Line,
                HashField::VulnIdFromTool
            ]
        );
    }

    #[test]
    fn unknown_scanner_falls_back_to_legacy() {
        let fields = fields_for_scanner("NonExistent Scanner Foo");
        assert_eq!(fields.len(), 5);
        assert!(fields.contains(&HashField::Title));
        assert!(fields.contains(&HashField::Description));
    }

    #[test]
    fn all_high_traffic_scanners_present() {
        // Each scanner we ship a parser for MUST have an explicit row.
        for s in [
            "Bandit Scan",
            "ZAP Scan",
            "Trivy Scan",
            "Semgrep JSON Report",
            "SARIF",
            "Snyk Scan",
            "Nuclei Scan",
        ] {
            let explicit = HASHCODE_FIELDS_PER_SCANNER
                .iter()
                .any(|(name, _)| *name == s);
            assert!(
                explicit,
                "scanner {s} missing from HASHCODE_FIELDS_PER_SCANNER"
            );
        }
    }

    #[test]
    fn hashfield_value_extraction() {
        let mut f = Finding::new("X", crate::finding::FindingSeverity::High);
        f.cwe = Some(79);
        f.file_path = Some("a.rs".into());
        assert_eq!(HashField::Title.value(&f), "X");
        assert_eq!(HashField::Cwe.value(&f), "79");
        assert_eq!(HashField::FilePath.value(&f), "a.rs");
        assert_eq!(HashField::Cve.value(&f), ""); // None ⇒ ""
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! License scanner.
//!
//! Mirrors trivy's `pkg/licensing` for cave-trivy MVP: parse common
//! LICENSE files, normalise SPDX identifiers, classify into category
//! buckets matching trivy's `--license-full` mapping (permissive,
//! weak-copyleft, copyleft, network-copyleft, restricted, forbidden,
//! unknown).

use crate::models::{License, LicenseCategory};

pub fn classify(spdx: &str) -> LicenseCategory {
    let s = spdx.trim().to_ascii_uppercase();
    if s.is_empty() {
        return LicenseCategory::Unknown;
    }
    if PERMISSIVE.iter().any(|x| *x == s) {
        return LicenseCategory::Permissive;
    }
    if WEAK_COPYLEFT.iter().any(|x| *x == s) {
        return LicenseCategory::Weakcopyleft;
    }
    if COPYLEFT.iter().any(|x| *x == s) {
        return LicenseCategory::Copyleft;
    }
    if NETWORK_COPYLEFT.iter().any(|x| *x == s) {
        return LicenseCategory::NetworkCopyleft;
    }
    if FORBIDDEN.iter().any(|x| *x == s) {
        return LicenseCategory::Forbidden;
    }
    if RESTRICTED.iter().any(|x| *x == s) {
        return LicenseCategory::Restricted;
    }
    LicenseCategory::Unknown
}

const PERMISSIVE: &[&str] = &[
    "MIT", "BSD-2-CLAUSE", "BSD-3-CLAUSE", "APACHE-2.0", "ISC", "ZLIB", "UNLICENSE", "BSL-1.0",
    "CC0-1.0", "0BSD", "MIT-0", "PYTHON-2.0",
];

const WEAK_COPYLEFT: &[&str] = &["LGPL-2.1-ONLY", "LGPL-2.1-OR-LATER", "LGPL-3.0-ONLY", "LGPL-3.0-OR-LATER", "MPL-2.0", "EPL-2.0", "EPL-1.0", "CDDL-1.0"];

const COPYLEFT: &[&str] = &["GPL-2.0-ONLY", "GPL-2.0-OR-LATER", "GPL-3.0-ONLY", "GPL-3.0-OR-LATER"];

const NETWORK_COPYLEFT: &[&str] = &["AGPL-3.0-ONLY", "AGPL-3.0-OR-LATER", "OSL-3.0"];

const RESTRICTED: &[&str] = &["CC-BY-NC-4.0", "CC-BY-NC-SA-4.0", "CC-BY-ND-4.0", "BUSL-1.1", "SSPL-1.0"];

const FORBIDDEN: &[&str] = &["JSON", "WTFPL", "BEER-WARE"];

/// Normalise a license header text into a single SPDX identifier guess.
/// Mirrors trivy's `pkg/licensing/expression` best-effort matcher.
pub fn detect_in_text(text: &str) -> Vec<String> {
    let upper = text.to_ascii_uppercase();
    let mut out = Vec::new();
    if upper.contains("APACHE LICENSE") && upper.contains("VERSION 2.0") {
        out.push("Apache-2.0".into());
    }
    if upper.contains("MIT LICENSE") || upper.contains("THE MIT LICENSE") {
        out.push("MIT".into());
    }
    if upper.contains("GNU GENERAL PUBLIC LICENSE") {
        if upper.contains("VERSION 3") {
            out.push("GPL-3.0-or-later".into());
        } else if upper.contains("VERSION 2") {
            out.push("GPL-2.0-or-later".into());
        }
    }
    if upper.contains("GNU AFFERO") {
        out.push("AGPL-3.0-or-later".into());
    }
    if upper.contains("GNU LESSER") {
        out.push("LGPL-3.0-or-later".into());
    }
    if upper.contains("MOZILLA PUBLIC LICENSE") {
        out.push("MPL-2.0".into());
    }
    if upper.contains("BSD") && upper.contains("REDISTRIBUTION") {
        out.push("BSD-3-Clause".into());
    }
    out
}

/// Build License records for a tree of `(path, body)` pairs.
pub fn scan_licenses(tree_files: &[(String, String)]) -> Vec<License> {
    let mut out = Vec::new();
    for (path, body) in tree_files {
        let base = path.rsplit('/').next().unwrap_or(path);
        if !is_license_file(base) {
            continue;
        }
        for id in detect_in_text(body) {
            let cat = classify(&id);
            out.push(License {
                pkg_name: path.clone(),
                license: id,
                category: cat,
                confidence: 80,
            });
        }
    }
    out
}

pub fn is_license_file(basename: &str) -> bool {
    let upper = basename.to_ascii_uppercase();
    matches!(
        upper.as_str(),
        "LICENSE" | "LICENSE.TXT" | "LICENSE.MD" | "COPYING" | "COPYING.TXT" | "NOTICE" | "NOTICE.TXT"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classification_table() {
        assert_eq!(classify("Apache-2.0"), LicenseCategory::Permissive);
        assert_eq!(classify("MIT"), LicenseCategory::Permissive);
        assert_eq!(classify("MPL-2.0"), LicenseCategory::Weakcopyleft);
        assert_eq!(classify("GPL-3.0-only"), LicenseCategory::Copyleft);
        assert_eq!(
            classify("AGPL-3.0-or-later"),
            LicenseCategory::NetworkCopyleft
        );
        assert_eq!(classify("BUSL-1.1"), LicenseCategory::Restricted);
        assert_eq!(classify("JSON"), LicenseCategory::Forbidden);
        assert_eq!(classify(""), LicenseCategory::Unknown);
        assert_eq!(classify("UNKNOWN"), LicenseCategory::Unknown);
    }

    #[test]
    fn detect_mit() {
        let t = "The MIT License\n\nPermission is hereby granted...";
        assert_eq!(detect_in_text(t)[0], "MIT");
    }

    #[test]
    fn detect_apache() {
        let t = "Apache License\nVersion 2.0, January 2004";
        assert_eq!(detect_in_text(t)[0], "Apache-2.0");
    }

    #[test]
    fn detect_agpl() {
        let t = "GNU AFFERO GENERAL PUBLIC LICENSE\nVersion 3";
        assert!(detect_in_text(t).contains(&"AGPL-3.0-or-later".to_string()));
    }

    #[test]
    fn scan_tree_finds_license_file() {
        let files = vec![(
            "LICENSE".to_string(),
            "MIT License\n\nPermission is hereby granted".to_string(),
        )];
        let v = scan_licenses(&files);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].license, "MIT");
        assert_eq!(v[0].category, LicenseCategory::Permissive);
    }

    #[test]
    fn ignores_non_license_files() {
        let files = vec![("README.md".to_string(), "MIT License".to_string())];
        assert!(scan_licenses(&files).is_empty());
    }

    #[test]
    fn is_license_file_basenames() {
        assert!(is_license_file("LICENSE"));
        assert!(is_license_file("COPYING"));
        assert!(is_license_file("NOTICE"));
        assert!(!is_license_file("README.md"));
    }
}

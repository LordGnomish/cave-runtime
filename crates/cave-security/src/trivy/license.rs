// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! License detection — SPDX license matching and risk classification.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LicenseFinding {
    pub file_path: String,
    pub license_id: String,
    pub license_name: String,
    pub risk_level: LicenseRisk,
    pub category: LicenseCategory,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LicenseRisk {
    /// GPL, AGPL — copyleft that could affect proprietary software
    High,
    /// LGPL, MPL, EUPL — weak copyleft
    Medium,
    /// MIT, Apache-2.0, BSD — permissive
    Low,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LicenseCategory {
    Permissive,
    WeakCopyleft,
    StrongCopyleft,
    NetworkCopyleft,
    PublicDomain,
    Proprietary,
    Unknown,
}

// ---------------------------------------------------------------------------
// SPDX license table
// ---------------------------------------------------------------------------

/// (spdx_id, display_name, risk, category)
pub static SPDX_LICENSES: &[(&str, &str, LicenseRisk, LicenseCategory)] = &[
    (
        "MIT",
        "MIT License",
        LicenseRisk::Low,
        LicenseCategory::Permissive,
    ),
    (
        "Apache-2.0",
        "Apache License 2.0",
        LicenseRisk::Low,
        LicenseCategory::Permissive,
    ),
    (
        "BSD-2-Clause",
        "BSD 2-Clause",
        LicenseRisk::Low,
        LicenseCategory::Permissive,
    ),
    (
        "BSD-3-Clause",
        "BSD 3-Clause",
        LicenseRisk::Low,
        LicenseCategory::Permissive,
    ),
    (
        "ISC",
        "ISC License",
        LicenseRisk::Low,
        LicenseCategory::Permissive,
    ),
    (
        "CC0-1.0",
        "Creative Commons Zero",
        LicenseRisk::Low,
        LicenseCategory::PublicDomain,
    ),
    (
        "Unlicense",
        "The Unlicense",
        LicenseRisk::Low,
        LicenseCategory::PublicDomain,
    ),
    (
        "0BSD",
        "Zero-Clause BSD",
        LicenseRisk::Low,
        LicenseCategory::Permissive,
    ),
    (
        "Zlib",
        "zlib License",
        LicenseRisk::Low,
        LicenseCategory::Permissive,
    ),
    (
        "MPL-2.0",
        "Mozilla Public License 2.0",
        LicenseRisk::Medium,
        LicenseCategory::WeakCopyleft,
    ),
    (
        "LGPL-2.0-only",
        "GNU LGPL v2.0",
        LicenseRisk::Medium,
        LicenseCategory::WeakCopyleft,
    ),
    (
        "LGPL-2.1-only",
        "GNU LGPL v2.1",
        LicenseRisk::Medium,
        LicenseCategory::WeakCopyleft,
    ),
    (
        "LGPL-3.0-only",
        "GNU LGPL v3.0",
        LicenseRisk::Medium,
        LicenseCategory::WeakCopyleft,
    ),
    (
        "EUPL-1.1",
        "European Union Public Licence 1.1",
        LicenseRisk::Medium,
        LicenseCategory::WeakCopyleft,
    ),
    (
        "EUPL-1.2",
        "European Union Public Licence 1.2",
        LicenseRisk::Medium,
        LicenseCategory::WeakCopyleft,
    ),
    (
        "GPL-2.0-only",
        "GNU GPL v2.0",
        LicenseRisk::High,
        LicenseCategory::StrongCopyleft,
    ),
    (
        "GPL-2.0-or-later",
        "GNU GPL v2.0+",
        LicenseRisk::High,
        LicenseCategory::StrongCopyleft,
    ),
    (
        "GPL-3.0-only",
        "GNU GPL v3.0",
        LicenseRisk::High,
        LicenseCategory::StrongCopyleft,
    ),
    (
        "GPL-3.0-or-later",
        "GNU GPL v3.0+",
        LicenseRisk::High,
        LicenseCategory::StrongCopyleft,
    ),
    (
        "AGPL-3.0-only",
        "GNU AGPL v3.0",
        LicenseRisk::High,
        LicenseCategory::NetworkCopyleft,
    ),
    (
        "AGPL-3.0-or-later",
        "GNU AGPL v3.0+",
        LicenseRisk::High,
        LicenseCategory::NetworkCopyleft,
    ),
    (
        "SSPL-1.0",
        "Server Side Public License",
        LicenseRisk::High,
        LicenseCategory::NetworkCopyleft,
    ),
];

/// Aliases used by packages in the wild → canonical SPDX id.
static LICENSE_ALIASES: &[(&str, &str)] = &[
    ("Apache 2", "Apache-2.0"),
    ("Apache 2.0", "Apache-2.0"),
    ("Apache License 2.0", "Apache-2.0"),
    ("Apache License, Version 2.0", "Apache-2.0"),
    ("MIT License", "MIT"),
    ("The MIT License", "MIT"),
    ("BSD", "BSD-3-Clause"),
    ("BSD License", "BSD-3-Clause"),
    ("2-Clause BSD", "BSD-2-Clause"),
    ("3-Clause BSD", "BSD-3-Clause"),
    ("New BSD License", "BSD-3-Clause"),
    ("Simplified BSD", "BSD-2-Clause"),
    ("GPL v2", "GPL-2.0-only"),
    ("GPL-2", "GPL-2.0-only"),
    ("GPLv2", "GPL-2.0-only"),
    ("GPLv3", "GPL-3.0-only"),
    ("GNU GPL", "GPL-3.0-only"),
    ("LGPL", "LGPL-2.1-only"),
    ("MPL", "MPL-2.0"),
    ("ISC License", "ISC"),
    ("Artistic License 2.0", "Artistic-2.0"),
];

// ---------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------

/// Normalise a raw license string to SPDX ID.
pub fn normalise_license(raw: &str) -> Option<&'static str> {
    let raw = raw.trim();
    // Direct SPDX match (case-insensitive)
    for (id, _name, _, _) in SPDX_LICENSES {
        if raw.eq_ignore_ascii_case(id) {
            return Some(id);
        }
    }
    // Alias match
    for (alias, canonical) in LICENSE_ALIASES {
        if raw.eq_ignore_ascii_case(alias) {
            return Some(canonical);
        }
    }
    None
}

/// Classify a raw license string and return a LicenseFinding.
pub fn classify_license(raw: &str, file_path: &str) -> LicenseFinding {
    let normalized = normalise_license(raw);
    if let Some(spdx_id) = normalized {
        // Find in table
        if let Some((id, name, risk, cat)) =
            SPDX_LICENSES.iter().find(|(id, _, _, _)| *id == spdx_id)
        {
            return LicenseFinding {
                file_path: file_path.to_string(),
                license_id: id.to_string(),
                license_name: name.to_string(),
                risk_level: risk.clone(),
                category: cat.clone(),
            };
        }
    }
    LicenseFinding {
        file_path: file_path.to_string(),
        license_id: raw.to_string(),
        license_name: raw.to_string(),
        risk_level: LicenseRisk::Unknown,
        category: LicenseCategory::Unknown,
    }
}

/// Detect license from a LICENSE file's content.
pub fn detect_from_license_file(content: &str, file_path: &str) -> Option<LicenseFinding> {
    // Heuristic fingerprinting
    let lower = content.to_lowercase();
    let id = if lower.contains("gnu affero general public license") {
        "AGPL-3.0-only"
    } else if lower.contains("gnu general public license") && lower.contains("version 3") {
        "GPL-3.0-only"
    } else if lower.contains("gnu general public license") && lower.contains("version 2") {
        "GPL-2.0-only"
    } else if lower.contains("gnu lesser general public license") && lower.contains("version 3") {
        "LGPL-3.0-only"
    } else if lower.contains("gnu lesser general public license") {
        "LGPL-2.1-only"
    } else if lower.contains("mozilla public license") {
        "MPL-2.0"
    } else if lower.contains("apache license") && lower.contains("version 2") {
        "Apache-2.0"
    } else if lower.contains("mit license")
        || (lower.contains("permission is hereby granted") && lower.contains("mit"))
    {
        "MIT"
    } else if lower.contains("isc license")
        || (lower.contains("isc") && lower.contains("permission to use"))
    {
        "ISC"
    } else if lower.contains("bsd 2-clause") || lower.contains("simplified bsd") {
        "BSD-2-Clause"
    } else if lower.contains("bsd 3-clause") || lower.contains("redistribution and use in source") {
        "BSD-3-Clause"
    } else if lower.contains("public domain") || lower.contains("cc0") {
        "CC0-1.0"
    } else if lower.contains("unlicense") {
        "Unlicense"
    } else {
        return None;
    };

    Some(classify_license(id, file_path))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalise_direct_spdx() {
        assert_eq!(normalise_license("MIT"), Some("MIT"));
        assert_eq!(normalise_license("Apache-2.0"), Some("Apache-2.0"));
        assert_eq!(normalise_license("GPL-3.0-only"), Some("GPL-3.0-only"));
    }

    #[test]
    fn normalise_alias() {
        assert_eq!(normalise_license("Apache 2.0"), Some("Apache-2.0"));
        assert_eq!(normalise_license("GPLv2"), Some("GPL-2.0-only"));
        assert_eq!(normalise_license("MIT License"), Some("MIT"));
    }

    #[test]
    fn classify_copyleft() {
        let f = classify_license("GPL-3.0-only", "/app/LICENSE");
        assert_eq!(f.risk_level, LicenseRisk::High);
        assert_eq!(f.category, LicenseCategory::StrongCopyleft);
    }

    #[test]
    fn classify_permissive() {
        let f = classify_license("MIT", "/app/LICENSE");
        assert_eq!(f.risk_level, LicenseRisk::Low);
    }

    #[test]
    fn detect_from_mit_text() {
        let content = "MIT License\nPermission is hereby granted, free of charge...";
        let f = detect_from_license_file(content, "LICENSE").unwrap();
        assert_eq!(f.license_id, "MIT");
    }

    #[test]
    fn detect_from_apache_text() {
        let content = "Apache License\nVersion 2.0, January 2004";
        let f = detect_from_license_file(content, "LICENSE").unwrap();
        assert_eq!(f.license_id, "Apache-2.0");
    }
}

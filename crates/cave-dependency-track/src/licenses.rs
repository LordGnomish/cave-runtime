// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! SPDX license catalog (curated subset of the SPDX 3.24 license-list-data).
//!
//! Mirrors `org.dependencytrack.persistence.DefaultObjectGenerator#loadDefaultLicenses`
//! — Dependency-Track also seeds a curated SPDX subset on first boot.

use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub struct SpdxLicense {
    pub spdx_id: &'static str,
    pub name: &'static str,
    pub osi_approved: bool,
    pub fsf_libre: bool,
    pub deprecated: bool,
}

const CATALOG: &[SpdxLicense] = &[
    SpdxLicense { spdx_id: "MIT", name: "MIT License", osi_approved: true, fsf_libre: true, deprecated: false },
    SpdxLicense { spdx_id: "Apache-2.0", name: "Apache License 2.0", osi_approved: true, fsf_libre: true, deprecated: false },
    SpdxLicense { spdx_id: "BSD-2-Clause", name: "BSD 2-Clause", osi_approved: true, fsf_libre: true, deprecated: false },
    SpdxLicense { spdx_id: "BSD-3-Clause", name: "BSD 3-Clause", osi_approved: true, fsf_libre: true, deprecated: false },
    SpdxLicense { spdx_id: "ISC", name: "ISC License", osi_approved: true, fsf_libre: true, deprecated: false },
    SpdxLicense { spdx_id: "GPL-2.0-only", name: "GNU GPL v2.0 only", osi_approved: true, fsf_libre: true, deprecated: false },
    SpdxLicense { spdx_id: "GPL-3.0-only", name: "GNU GPL v3.0 only", osi_approved: true, fsf_libre: true, deprecated: false },
    SpdxLicense { spdx_id: "GPL-3.0-or-later", name: "GNU GPL v3.0 or later", osi_approved: true, fsf_libre: true, deprecated: false },
    SpdxLicense { spdx_id: "LGPL-2.1-or-later", name: "GNU LGPL v2.1 or later", osi_approved: true, fsf_libre: true, deprecated: false },
    SpdxLicense { spdx_id: "LGPL-3.0-or-later", name: "GNU LGPL v3.0 or later", osi_approved: true, fsf_libre: true, deprecated: false },
    SpdxLicense { spdx_id: "AGPL-3.0-only", name: "GNU AGPL v3.0 only", osi_approved: true, fsf_libre: true, deprecated: false },
    SpdxLicense { spdx_id: "AGPL-3.0-or-later", name: "GNU AGPL v3.0 or later", osi_approved: true, fsf_libre: true, deprecated: false },
    SpdxLicense { spdx_id: "MPL-2.0", name: "Mozilla Public License 2.0", osi_approved: true, fsf_libre: true, deprecated: false },
    SpdxLicense { spdx_id: "EPL-2.0", name: "Eclipse Public License 2.0", osi_approved: true, fsf_libre: true, deprecated: false },
    SpdxLicense { spdx_id: "CDDL-1.1", name: "CDDL 1.1", osi_approved: true, fsf_libre: false, deprecated: false },
    SpdxLicense { spdx_id: "Unlicense", name: "The Unlicense", osi_approved: true, fsf_libre: true, deprecated: false },
    SpdxLicense { spdx_id: "0BSD", name: "BSD Zero Clause", osi_approved: true, fsf_libre: true, deprecated: false },
    SpdxLicense { spdx_id: "Zlib", name: "zlib License", osi_approved: true, fsf_libre: true, deprecated: false },
    SpdxLicense { spdx_id: "WTFPL", name: "WTFPL", osi_approved: false, fsf_libre: true, deprecated: false },
    SpdxLicense { spdx_id: "Artistic-2.0", name: "Artistic License 2.0", osi_approved: true, fsf_libre: true, deprecated: false },
    SpdxLicense { spdx_id: "OpenSSL", name: "OpenSSL License", osi_approved: false, fsf_libre: false, deprecated: false },
    SpdxLicense { spdx_id: "GPL-2.0", name: "GNU GPL v2.0 (deprecated)", osi_approved: true, fsf_libre: true, deprecated: true },
    SpdxLicense { spdx_id: "GPL-3.0", name: "GNU GPL v3.0 (deprecated)", osi_approved: true, fsf_libre: true, deprecated: true },
];

pub fn catalog() -> &'static [SpdxLicense] {
    CATALOG
}

pub fn lookup(spdx_id: &str) -> Option<&'static SpdxLicense> {
    CATALOG.iter().find(|l| l.spdx_id.eq_ignore_ascii_case(spdx_id))
}

pub fn is_known(spdx_id: &str) -> bool {
    lookup(spdx_id).is_some()
}

pub fn osi_approved(spdx_id: &str) -> bool {
    lookup(spdx_id).map(|l| l.osi_approved).unwrap_or(false)
}

pub fn build_index() -> HashMap<&'static str, &'static SpdxLicense> {
    CATALOG.iter().map(|l| (l.spdx_id, l)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_contains_baseline() {
        assert!(is_known("MIT"));
        assert!(is_known("Apache-2.0"));
        assert!(is_known("AGPL-3.0-or-later"));
    }

    #[test]
    fn lookup_case_insensitive() {
        assert!(lookup("mit").is_some());
        assert!(lookup("apache-2.0").is_some());
    }

    #[test]
    fn osi_classification_for_mit() {
        assert!(osi_approved("MIT"));
    }

    #[test]
    fn deprecated_short_form_flagged() {
        assert!(lookup("GPL-3.0").unwrap().deprecated);
        assert!(!lookup("GPL-3.0-only").unwrap().deprecated);
    }

    #[test]
    fn build_index_round_trip() {
        let idx = build_index();
        assert!(idx.contains_key("MIT"));
        assert_eq!(idx.get("MIT").unwrap().name, "MIT License");
    }

    #[test]
    fn catalog_size_minimum() {
        assert!(catalog().len() >= 20);
    }
}

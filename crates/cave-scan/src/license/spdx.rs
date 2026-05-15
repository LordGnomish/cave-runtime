// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: aquasecurity/trivy@8a3177a pkg/licensing/classifier.go
//! SPDX detection — header parse + canonical-wording fingerprints.

use super::{License, LicenseDetector};
use regex::Regex;
use std::sync::OnceLock;

/// One canonical-text fingerprint.
#[derive(Debug, Clone, Copy)]
struct Fingerprint {
    spdx_id: &'static str,
    needle: &'static str,
    is_copyleft: bool,
}

/// Built-in fingerprint library — canonical license header phrases.
const FINGERPRINTS: &[Fingerprint] = &[
    Fingerprint {
        spdx_id: "MIT",
        needle: "Permission is hereby granted, free of charge",
        is_copyleft: false,
    },
    Fingerprint {
        spdx_id: "Apache-2.0",
        needle: "Apache License",
        is_copyleft: false,
    },
    Fingerprint {
        spdx_id: "BSD-3-Clause",
        needle: "Redistributions in binary form must reproduce the above",
        is_copyleft: false,
    },
    Fingerprint {
        spdx_id: "BSD-2-Clause",
        needle: "Redistribution and use in source and binary forms",
        is_copyleft: false,
    },
    Fingerprint {
        spdx_id: "ISC",
        needle: "Permission to use, copy, modify, and/or distribute this software",
        is_copyleft: false,
    },
    Fingerprint {
        spdx_id: "AGPL-3.0",
        needle: "GNU AFFERO GENERAL PUBLIC LICENSE",
        is_copyleft: true,
    },
    Fingerprint {
        spdx_id: "GPL-3.0",
        needle: "GNU GENERAL PUBLIC LICENSE",
        is_copyleft: true,
    },
    Fingerprint {
        spdx_id: "GPL-2.0",
        needle: "GNU General Public License, version 2",
        is_copyleft: true,
    },
    Fingerprint {
        spdx_id: "LGPL-3.0",
        needle: "GNU LESSER GENERAL PUBLIC LICENSE",
        is_copyleft: true,
    },
    Fingerprint {
        spdx_id: "LGPL-2.1",
        needle: "GNU Lesser General Public License, version 2.1",
        is_copyleft: true,
    },
    Fingerprint {
        spdx_id: "MPL-2.0",
        needle: "Mozilla Public License Version 2.0",
        is_copyleft: true,
    },
    Fingerprint {
        spdx_id: "EPL-2.0",
        needle: "Eclipse Public License - v 2.0",
        is_copyleft: true,
    },
    Fingerprint {
        spdx_id: "EPL-1.0",
        needle: "Eclipse Public License - v 1.0",
        is_copyleft: true,
    },
    Fingerprint {
        spdx_id: "Unlicense",
        needle: "This is free and unencumbered software released into the public domain",
        is_copyleft: false,
    },
];

/// Cached compiled regex for SPDX header detection.
fn spdx_header_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"SPDX-License-Identifier:\s*([A-Za-z0-9\.\-\+]+)")
            .expect("spdx header regex must compile")
    })
}

fn is_copyleft_id(id: &str) -> bool {
    let up = id.to_ascii_uppercase();
    up.starts_with("GPL")
        || up.starts_with("AGPL")
        || up.starts_with("LGPL")
        || up.starts_with("MPL")
        || up.starts_with("EPL")
        || up.starts_with("CDDL")
        || up.starts_with("SSPL")
}

/// License scanner with the built-in fingerprint library.
#[derive(Default, Clone)]
pub struct LicenseScanner;

impl LicenseScanner {
    pub fn new() -> Self {
        Self
    }
}

impl LicenseDetector for LicenseScanner {
    fn detect_from_text(&self, content: &str) -> Vec<License> {
        let mut out = Vec::new();
        // 1. SPDX header
        for cap in spdx_header_re().captures_iter(content) {
            let id = cap[1].to_string();
            let is_cl = is_copyleft_id(&id);
            out.push(License {
                spdx_id: id,
                source: "header".into(),
                is_copyleft: is_cl,
            });
        }
        // 2. Canonical-wording fingerprints
        for fp in FINGERPRINTS {
            if content.contains(fp.needle) {
                // Avoid double-counting if SPDX header already saw it
                if !out.iter().any(|l| l.spdx_id == fp.spdx_id) {
                    out.push(License {
                        spdx_id: fp.spdx_id.to_string(),
                        source: "fingerprint".into(),
                        is_copyleft: fp.is_copyleft,
                    });
                }
            }
        }
        out
    }

    fn is_license_path(&self, path: &str) -> bool {
        let lc = path.to_ascii_lowercase();
        let base = lc.rsplit('/').next().unwrap_or(&lc);
        let stem = base.split('.').next().unwrap_or(base);
        matches!(stem, "license" | "copying" | "notice" | "licence")
    }
}

impl LicenseScanner {
    pub fn detect_from_text(&self, content: &str) -> Vec<License> {
        <Self as LicenseDetector>::detect_from_text(self, content)
    }
    pub fn is_license_path(&self, path: &str) -> bool {
        <Self as LicenseDetector>::is_license_path(self, path)
    }
}

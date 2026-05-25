// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: aquasecurity/trivy@8a3177a pkg/licensing/scanner.go
//! License scanner.
//!
//! Two-mode detection:
//!
//! 1. **Path heuristic** — files named LICENSE / COPYING / NOTICE in any case.
//! 2. **Content scan** — `SPDX-License-Identifier:` header parse, plus a small
//!    library of license-text fingerprints (MIT, Apache-2.0, BSD-3-Clause,
//!    GPL-2.0/3.0, AGPL-3.0, LGPL-2.1/3.0, MPL-2.0, EPL-1.0/2.0, ISC).
//!
//! Trivy's upstream uses `google/licensecheck` (rabin-fingerprint hashing) for
//! full corpus matching — we use cheaper substring heuristics that catch the
//! canonical wording of each license header. See parity manifest `missing`.

pub mod spdx;

/// One detected license.
#[derive(Debug, Clone, PartialEq)]
pub struct License {
    /// SPDX short identifier, e.g. "MIT", "Apache-2.0".
    pub spdx_id: String,
    /// Detection method ("path", "header", "fingerprint").
    pub source: String,
    /// True for GPL/AGPL/LGPL/MPL/EPL.
    pub is_copyleft: bool,
}

/// License scanner contract.
pub trait LicenseDetector {
    fn detect_from_text(&self, content: &str) -> Vec<License>;
    fn is_license_path(&self, path: &str) -> bool;
}

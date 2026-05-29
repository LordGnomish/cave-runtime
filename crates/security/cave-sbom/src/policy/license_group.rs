// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: DependencyTrack/dependency-track@128fd0fa01bed9fcb57abffa3b30047c45941415
//   src/main/java/org/dependencytrack/policy/LicenseGroupPolicyEvaluator.java
//
//! License-group evaluator — checks whether a component's SPDX license
//! belongs to a named group (Permissive / Copyleft / WeakCopyleft / Proprietary).
//!
//! Mirrors DependencyTrack's `LicenseGroup` roll-up used by
//! `LicenseGroupPolicyEvaluator`.

use crate::components::ComponentRecord;

/// Canonical license groups mirroring DependencyTrack's `LicenseGroup` model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LicenseGroup {
    /// MIT, Apache-2.0, BSD-*, ISC, Unlicense, etc.
    Permissive,
    /// GPL-2.0, GPL-3.0, AGPL-3.0, EUPL, etc.
    Copyleft,
    /// LGPL-2.1, LGPL-3.0, MPL-2.0, CDDL, EUPL (limited), etc.
    WeakCopyleft,
    /// Commercial / proprietary identifiers.
    Proprietary,
}

impl LicenseGroup {
    /// Human-readable name matching the DependencyTrack UI group names.
    pub fn as_str(&self) -> &'static str {
        match self {
            LicenseGroup::Permissive => "Permissive",
            LicenseGroup::Copyleft => "Copyleft",
            LicenseGroup::WeakCopyleft => "WeakCopyleft",
            LicenseGroup::Proprietary => "Proprietary",
        }
    }
}

impl std::fmt::Display for LicenseGroup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Known license group → SPDX ID mappings.
///
/// The list is curated to match DependencyTrack's bundled `license-groups.json`
/// for the most common SPDX identifiers. Case-insensitive lookup is performed
/// at call-sites via `license_belongs_to_group`.
pub const KNOWN_GROUPS: &[(&str, LicenseGroup)] = &[
    // ── Permissive ───────────────────────────────────────────────────────────
    ("MIT", LicenseGroup::Permissive),
    ("MIT-0", LicenseGroup::Permissive),
    ("Apache-2.0", LicenseGroup::Permissive),
    ("Apache-1.0", LicenseGroup::Permissive),
    ("Apache-1.1", LicenseGroup::Permissive),
    ("BSD-2-Clause", LicenseGroup::Permissive),
    ("BSD-3-Clause", LicenseGroup::Permissive),
    ("BSD-4-Clause", LicenseGroup::Permissive),
    ("ISC", LicenseGroup::Permissive),
    ("Zlib", LicenseGroup::Permissive),
    ("0BSD", LicenseGroup::Permissive),
    ("Unlicense", LicenseGroup::Permissive),
    ("CC0-1.0", LicenseGroup::Permissive),
    ("PSF-2.0", LicenseGroup::Permissive),
    ("Python-2.0", LicenseGroup::Permissive),
    ("Artistic-2.0", LicenseGroup::Permissive),
    ("WTFPL", LicenseGroup::Permissive),
    ("BlueOak-1.0.0", LicenseGroup::Permissive),
    ("Boost-1.0", LicenseGroup::Permissive),
    // ── Copyleft ─────────────────────────────────────────────────────────────
    ("GPL-2.0", LicenseGroup::Copyleft),
    ("GPL-2.0-only", LicenseGroup::Copyleft),
    ("GPL-2.0-or-later", LicenseGroup::Copyleft),
    ("GPL-3.0", LicenseGroup::Copyleft),
    ("GPL-3.0-only", LicenseGroup::Copyleft),
    ("GPL-3.0-or-later", LicenseGroup::Copyleft),
    ("AGPL-3.0", LicenseGroup::Copyleft),
    ("AGPL-3.0-only", LicenseGroup::Copyleft),
    ("AGPL-3.0-or-later", LicenseGroup::Copyleft),
    ("EUPL-1.1", LicenseGroup::Copyleft),
    ("EUPL-1.2", LicenseGroup::Copyleft),
    ("OSL-3.0", LicenseGroup::Copyleft),
    ("SSPL-1.0", LicenseGroup::Copyleft),
    // ── WeakCopyleft ─────────────────────────────────────────────────────────
    ("LGPL-2.0", LicenseGroup::WeakCopyleft),
    ("LGPL-2.0-only", LicenseGroup::WeakCopyleft),
    ("LGPL-2.0-or-later", LicenseGroup::WeakCopyleft),
    ("LGPL-2.1", LicenseGroup::WeakCopyleft),
    ("LGPL-2.1-only", LicenseGroup::WeakCopyleft),
    ("LGPL-2.1-or-later", LicenseGroup::WeakCopyleft),
    ("LGPL-3.0", LicenseGroup::WeakCopyleft),
    ("LGPL-3.0-only", LicenseGroup::WeakCopyleft),
    ("LGPL-3.0-or-later", LicenseGroup::WeakCopyleft),
    ("MPL-1.1", LicenseGroup::WeakCopyleft),
    ("MPL-2.0", LicenseGroup::WeakCopyleft),
    ("CDDL-1.0", LicenseGroup::WeakCopyleft),
    ("CDDL-1.1", LicenseGroup::WeakCopyleft),
    ("EPL-1.0", LicenseGroup::WeakCopyleft),
    ("EPL-2.0", LicenseGroup::WeakCopyleft),
    ("CPL-1.0", LicenseGroup::WeakCopyleft),
    ("APSL-2.0", LicenseGroup::WeakCopyleft),
    // ── Proprietary ──────────────────────────────────────────────────────────
    ("LicenseRef-Commercial", LicenseGroup::Proprietary),
    ("LicenseRef-Proprietary", LicenseGroup::Proprietary),
    ("LicenseRef-MSFT", LicenseGroup::Proprietary),
];

/// Parse a group name string (case-insensitive) to a `LicenseGroup` variant.
///
/// Returns `None` if the name is not recognized.
pub fn parse_group_name(name: &str) -> Option<LicenseGroup> {
    match name.to_lowercase().as_str() {
        "permissive" => Some(LicenseGroup::Permissive),
        "copyleft" => Some(LicenseGroup::Copyleft),
        "weakcopyleft" | "weak_copyleft" | "weak-copyleft" => Some(LicenseGroup::WeakCopyleft),
        "proprietary" => Some(LicenseGroup::Proprietary),
        _ => None,
    }
}

/// Returns `true` if the given SPDX license identifier belongs to `group`.
///
/// Matching is case-insensitive on the SPDX identifier.
pub fn license_belongs_to_group(spdx_id: &str, group: LicenseGroup) -> bool {
    let lower = spdx_id.to_lowercase();
    KNOWN_GROUPS
        .iter()
        .any(|(id, g)| *g == group && id.to_lowercase() == lower)
}

/// Evaluate the `LicenseInGroup` condition for a single component.
///
/// Returns `Some(message)` when the component's license belongs to the
/// named group; `None` otherwise (no violation).
pub fn violates_license_in_group(c: &ComponentRecord, group_name: &str) -> Option<String> {
    let license = c.license.as_deref()?;
    let group = parse_group_name(group_name)?;
    if license_belongs_to_group(license, group) {
        Some(format!(
            "{} uses license {} which belongs to the {} group",
            c.name, license, group
        ))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mit_is_permissive() {
        assert!(license_belongs_to_group("MIT", LicenseGroup::Permissive));
    }

    #[test]
    fn gpl3_is_copyleft() {
        assert!(license_belongs_to_group("GPL-3.0", LicenseGroup::Copyleft));
    }

    #[test]
    fn lgpl21_is_weakcopyleft() {
        assert!(license_belongs_to_group("LGPL-2.1", LicenseGroup::WeakCopyleft));
    }

    #[test]
    fn unknown_is_no_group() {
        assert!(!license_belongs_to_group("SomeUnknownLicense", LicenseGroup::Permissive));
    }

    #[test]
    fn case_insensitive_match() {
        assert!(license_belongs_to_group("mit", LicenseGroup::Permissive));
        assert!(license_belongs_to_group("MIT", LicenseGroup::Permissive));
        assert!(license_belongs_to_group("Apache-2.0", LicenseGroup::Permissive));
    }

    #[test]
    fn parse_group_name_case_insensitive() {
        assert_eq!(parse_group_name("Copyleft"), Some(LicenseGroup::Copyleft));
        assert_eq!(parse_group_name("copyleft"), Some(LicenseGroup::Copyleft));
        assert_eq!(parse_group_name("PERMISSIVE"), Some(LicenseGroup::Permissive));
    }

    #[test]
    fn parse_unknown_group_is_none() {
        assert!(parse_group_name("unknown-group").is_none());
    }
}

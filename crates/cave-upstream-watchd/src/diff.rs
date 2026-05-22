// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Version diff — `parity.manifest` pin vs `release.tag_name`.
//!
//! Both sides are normalised before comparison (`v` prefix stripped,
//! pre-release suffixes trimmed) so `v1.36.0` and `1.36.0-rc.1` end
//! up with comparable major.minor.patch tuples.
//!
//! Falls back to lexicographic compare when either side is not
//! semver-shaped (a hash, a date tag, …). The fallback flags the
//! diff as `Severity::Unknown` so the dispatcher doesn't auto-port
//! against a comparison it can't grade.

use semver::Version;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Pin already at-or-ahead of upstream.
    None,
    /// Patch-only difference (z digit moved).
    Patch,
    /// Minor difference (y digit moved).
    Minor,
    /// Major difference (x digit moved).
    Major,
    /// Either side wasn't semver-shaped.
    Unknown,
}

/// Outcome of `compare_pin_against_latest`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionDiff {
    pub pin: Option<String>,
    pub latest: String,
    pub severity: Severity,
    /// `true` when `pin < latest` semver-wise; `false` when
    /// already at parity (or pin ahead).
    pub outdated: bool,
}

/// Compare a local pin against an upstream release tag.
///
/// * `pin` — the value from `parity.manifest.toml::[upstream] version`.
///   May be `None` for a fresh crate that hasn't pinned anything yet.
/// * `latest` — the release tag from GitHub's `tag_name`, e.g.
///   `"v1.36.0"` or `"3.5.13"`.
///
/// Returns a `VersionDiff` so the daemon can render the result + the
/// dispatcher can route on severity.
pub fn compare_pin_against_latest(pin: Option<&str>, latest: &str) -> VersionDiff {
    let pin_v = pin.and_then(parse_version);
    let lat_v = parse_version(latest);

    match (&pin_v, &lat_v) {
        (Some(p), Some(l)) => {
            if l <= p {
                VersionDiff {
                    pin: pin.map(str::to_string),
                    latest: latest.to_string(),
                    severity: Severity::None,
                    outdated: false,
                }
            } else if l.major > p.major {
                VersionDiff {
                    pin: pin.map(str::to_string),
                    latest: latest.to_string(),
                    severity: Severity::Major,
                    outdated: true,
                }
            } else if l.minor > p.minor {
                VersionDiff {
                    pin: pin.map(str::to_string),
                    latest: latest.to_string(),
                    severity: Severity::Minor,
                    outdated: true,
                }
            } else {
                VersionDiff {
                    pin: pin.map(str::to_string),
                    latest: latest.to_string(),
                    severity: Severity::Patch,
                    outdated: true,
                }
            }
        }
        // No pin recorded → treat as outdated (we want to surface a
        // gap so the operator can pin); severity is Unknown because
        // we don't know how far the upstream has moved.
        (None, Some(_)) => VersionDiff {
            pin: None,
            latest: latest.to_string(),
            severity: Severity::Unknown,
            outdated: true,
        },
        // Latest didn't parse — lexicographic fallback.
        (Some(_), None) | (None, None) => {
            let outdated = match pin {
                Some(p) => p != latest,
                None => true,
            };
            VersionDiff {
                pin: pin.map(str::to_string),
                latest: latest.to_string(),
                severity: Severity::Unknown,
                outdated,
            }
        }
    }
}

/// Best-effort parse to a `semver::Version`.
///
/// Trims a leading `v`/`V`, drops anything after the first `+` or `-`
/// that doesn't look like a numeric pre-release suffix (so
/// `1.36.0-rc.1` parses as 1.36.0). Returns `None` for non-semver
/// shapes like `release-2026-05-01` or commit SHAs.
fn parse_version(s: &str) -> Option<Version> {
    let s = s.trim();
    let s = s.strip_prefix(['v', 'V']).unwrap_or(s);
    // Try as-is first; semver supports pre-release suffixes natively.
    if let Ok(v) = Version::parse(s) {
        return Some(v);
    }
    // Drop everything from the first `-` or `+` and retry.
    let head = s.split(|c| c == '-' || c == '+').next()?;
    Version::parse(head).ok().or_else(|| {
        // Some upstreams ship `1.36` (no patch). Pad with `.0`.
        let dots = head.matches('.').count();
        if dots == 1 {
            Version::parse(&format!("{head}.0")).ok()
        } else if dots == 0 {
            Version::parse(&format!("{head}.0.0")).ok()
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pin_equal_to_latest_is_at_parity() {
        let d = compare_pin_against_latest(Some("v1.36.0"), "v1.36.0");
        assert_eq!(d.severity, Severity::None);
        assert!(!d.outdated);
    }

    #[test]
    fn pin_ahead_of_latest_is_at_parity() {
        let d = compare_pin_against_latest(Some("v2.0.0"), "v1.36.0");
        assert_eq!(d.severity, Severity::None);
        assert!(!d.outdated);
    }

    #[test]
    fn patch_bump_only_flags_patch() {
        let d = compare_pin_against_latest(Some("v1.36.0"), "v1.36.5");
        assert_eq!(d.severity, Severity::Patch);
        assert!(d.outdated);
    }

    #[test]
    fn minor_bump_flags_minor() {
        let d = compare_pin_against_latest(Some("v1.36.0"), "v1.37.0");
        assert_eq!(d.severity, Severity::Minor);
        assert!(d.outdated);
    }

    #[test]
    fn major_bump_flags_major() {
        let d = compare_pin_against_latest(Some("v1.36.0"), "v2.0.0");
        assert_eq!(d.severity, Severity::Major);
        assert!(d.outdated);
    }

    #[test]
    fn v_prefix_does_not_affect_compare() {
        assert_eq!(
            compare_pin_against_latest(Some("v1.36.0"), "1.36.0").severity,
            Severity::None,
        );
    }

    #[test]
    fn pre_release_suffix_is_trimmed_for_compare() {
        // Pre-release suffix means "before the GA": v1.37.0-rc.1 < v1.37.0.
        // Our normaliser drops the suffix to get 1.37.0 == 1.37.0 → no diff
        // between 1.37.0-rc.1 and 1.37.0.
        let d = compare_pin_against_latest(Some("v1.37.0"), "v1.37.0-rc.1");
        assert_eq!(d.severity, Severity::None);
    }

    #[test]
    fn missing_pin_is_unknown_outdated() {
        let d = compare_pin_against_latest(None, "v1.0.0");
        assert_eq!(d.severity, Severity::Unknown);
        assert!(d.outdated);
        assert!(d.pin.is_none());
    }

    #[test]
    fn non_semver_latest_falls_back_to_lex_with_unknown_severity() {
        let d = compare_pin_against_latest(Some("v1.0.0"), "release-2026-05-13");
        assert_eq!(d.severity, Severity::Unknown);
        // pin parses, latest doesn't → unknown, outdated=true because they differ.
        assert!(d.outdated);
    }

    #[test]
    fn two_part_version_padded_to_x_y_zero() {
        // Some upstreams ship `1.40` (no patch).
        let d = compare_pin_against_latest(Some("1.40"), "1.40.0");
        assert_eq!(d.severity, Severity::None);
    }
}

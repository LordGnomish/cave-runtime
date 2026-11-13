// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: aquasecurity/trivy-db@2034dd8 pkg/utils/utils.go
//! Go module pseudo-version normalisation and comparison.
//!
//! Go module pseudo-versions (https://go.dev/ref/mod#pseudo-versions) encode
//! a commit timestamp and short hash in the version string, e.g.:
//!
//! * `v0.0.0-20210423082822-c015be86a520`    — standard form (base 0.0.0)
//! * `v1.2.0-0.20220101120000-deadbeefcafe`  — pre-release form (patched base)
//! * `v2.3.4-0.20230601000000-aabbccddeeff`  — tagged pre-release form
//!
//! Comparison is purely chronological: the 14-digit yyyymmddhhmmss timestamp
//! is the sort key.
//!
//! A real tagged version (e.g. `v1.0.0`) ranks higher than a pseudo-version
//! with a `v0.x` base, because the pseudo-version pre-dates the tag.

use regex::Regex;
use std::sync::OnceLock;

fn pseudo_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // Pre-release form: vX.Y.Z-0.yyyymmddhhmmss-abcdefabcdef
        Regex::new(r"^v\d+\.\d+\.\d+-0\.(\d{14})-[0-9a-f]{12}$").expect("valid regex")
    })
}

fn std_pseudo_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // Standard form: v0.0.0-yyyymmddhhmmss-abcdefabcdef
        Regex::new(r"^v\d+\.\d+\.\d+-(\d{14})-[0-9a-f]{12}$").expect("valid regex")
    })
}

/// Returns `true` if `version` is a Go module pseudo-version.
///
/// Accepted forms (Go spec §module-pseudo-version):
/// * `v0.0.0-yyyymmddhhmmss-abcdefabcdef`           (standard)
/// * `vX.Y.0-0.yyyymmddhhmmss-abcdefabcdef`         (pre-release, patched base)
/// * `vX.Y.Z-0.yyyymmddhhmmss-abcdefabcdef`         (tagged pre-release)
pub fn is_pseudo_version(version: &str) -> bool {
    pseudo_re().is_match(version) || std_pseudo_re().is_match(version)
}

/// Extract the 14-digit timestamp from a pseudo-version string.
///
/// Returns the yyyymmddhhmmss segment.  If the input is not a pseudo-version,
/// returns the input string unchanged (pass-through).
pub fn normalize_pseudo_version(version: &str) -> &str {
    if let Some(caps) = pseudo_re().captures(version) {
        return caps.get(1).map(|m| m.as_str()).unwrap_or(version);
    }
    if let Some(caps) = std_pseudo_re().captures(version) {
        return caps.get(1).map(|m| m.as_str()).unwrap_or(version);
    }
    version
}

/// Compare two Go module versions, handling pseudo-versions correctly.
///
/// Returns:
/// * `< 0` if `a` is older/lower than `b`
/// * `0`   if they are considered equal
/// * `> 0` if `a` is newer/higher than `b`
///
/// Ordering rules:
/// 1. Two pseudo-versions → compare timestamps as 14-char strings (ISO order).
/// 2. Pseudo vs real tag → compare the pseudo's base version against the tag.
/// 3. Two real tags → element-wise numeric semver comparison.
pub fn pseudo_version_cmp(a: &str, b: &str) -> i32 {
    let a_pseudo = is_pseudo_version(a);
    let b_pseudo = is_pseudo_version(b);

    match (a_pseudo, b_pseudo) {
        (true, true) => {
            let ts_a = normalize_pseudo_version(a);
            let ts_b = normalize_pseudo_version(b);
            // 14-digit ISO timestamp strings sort correctly as ASCII.
            match ts_a.cmp(ts_b) {
                std::cmp::Ordering::Less => -1,
                std::cmp::Ordering::Equal => 0,
                std::cmp::Ordering::Greater => 1,
            }
        }
        (true, false) => {
            // Pseudo a vs real b: compare pseudo's base against b.
            let base_a = pseudo_base_version(a);
            cmp_semver(base_a, b)
        }
        (false, true) => {
            // Real a vs pseudo b: compare a against pseudo's base.
            let base_b = pseudo_base_version(b);
            cmp_semver(a, base_b)
        }
        (false, false) => cmp_semver(a, b),
    }
}

/// Extract the base version (everything before the first `-`) from a
/// pseudo-version string.
fn pseudo_base_version(v: &str) -> &str {
    if let Some(idx) = v.find('-') {
        &v[..idx]
    } else {
        v
    }
}

/// Element-wise numeric semver comparison (strips leading `v`).
fn cmp_semver(a: &str, b: &str) -> i32 {
    let a = a.trim_start_matches('v');
    let b = b.trim_start_matches('v');
    let pa: Vec<u64> = a.split('.').map(|s| s.parse().unwrap_or(0)).collect();
    let pb: Vec<u64> = b.split('.').map(|s| s.parse().unwrap_or(0)).collect();
    let n = pa.len().max(pb.len());
    for i in 0..n {
        let ai = pa.get(i).copied().unwrap_or(0);
        let bi = pb.get(i).copied().unwrap_or(0);
        if ai < bi {
            return -1;
        }
        if ai > bi {
            return 1;
        }
    }
    0
}

#[cfg(test)]
mod unit {
    use super::*;

    #[test]
    fn base_extraction_standard() {
        assert_eq!(
            pseudo_base_version("v0.0.0-20210101000000-aabbccddeeff"),
            "v0.0.0"
        );
    }

    #[test]
    fn base_extraction_pre_release() {
        assert_eq!(
            pseudo_base_version("v1.2.0-0.20220101120000-deadbeefcafe"),
            "v1.2.0"
        );
    }

    #[test]
    fn cmp_semver_basic() {
        assert_eq!(cmp_semver("1.0.0", "1.0.0"), 0);
        assert_eq!(cmp_semver("2.0.0", "1.99.0"), 1);
        assert_eq!(cmp_semver("v0.0.0", "v1.0.0"), -1);
    }
}

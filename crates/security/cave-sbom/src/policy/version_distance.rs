// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: DependencyTrack/dependency-track@128fd0fa01bed9fcb57abffa3b30047c45941415
//   src/main/java/org/dependencytrack/policy/VersionDistancePolicyEvaluator.java
//
//! Version-distance evaluator — checks how many published versions behind
//! a component's current version is from the latest known release.
//!
//! Mirrors DependencyTrack's `VersionDistancePolicyEvaluator`.

use crate::components::{ComponentRecord, version_compare};

/// Returns the number of versions in `available_versions` that are strictly
/// newer than `component_version`.
///
/// The available versions are sorted ascending using the same
/// `version_compare` function as the rest of the crate.  If
/// `component_version` does not appear in the list the component is
/// assumed to be older than all known versions and `available.len()` is
/// returned.
pub fn versions_behind(component_version: &str, available_versions: &[&str]) -> u32 {
    use std::cmp::Ordering;

    if available_versions.is_empty() {
        return 0;
    }

    // Sort ascending (smallest first).
    let mut sorted: Vec<&str> = available_versions.to_vec();
    sorted.sort_by(|a, b| version_compare(a, b));
    sorted.dedup();

    let latest = sorted[sorted.len() - 1];

    // If already at latest, 0 behind.
    if version_compare(component_version, latest) == Ordering::Equal {
        return 0;
    }

    // Count how many available versions are strictly newer than component_version.
    let newer_count = sorted
        .iter()
        .filter(|v| version_compare(v, component_version) == Ordering::Greater)
        .count();

    newer_count as u32
}

/// Evaluate the `VersionDistanceAtLeast` condition for a single component.
///
/// Returns `Some(message)` when the component is strictly more than
/// `max_behind` versions behind the latest in `available_versions`.
pub fn violates_version_distance(
    c: &ComponentRecord,
    max_behind: u32,
    available_versions: &[String],
) -> Option<String> {
    let refs: Vec<&str> = available_versions.iter().map(|s| s.as_str()).collect();
    let behind = versions_behind(&c.version, &refs);
    if behind > max_behind {
        Some(format!(
            "{} {} is {} version(s) behind the latest (threshold: {})",
            c.name, c.version, behind, max_behind
        ))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn at_latest_is_zero() {
        let avail = vec!["1.0.0", "2.0.0", "3.0.0"];
        assert_eq!(versions_behind("3.0.0", &avail), 0);
    }

    #[test]
    fn one_behind() {
        let avail = vec!["1.0.0", "2.0.0", "3.0.0"];
        assert_eq!(versions_behind("2.0.0", &avail), 1);
    }

    #[test]
    fn two_behind() {
        let avail = vec!["1.0.0", "2.0.0", "3.0.0"];
        assert_eq!(versions_behind("1.0.0", &avail), 2);
    }

    #[test]
    fn unknown_version_is_max_behind() {
        let avail = vec!["2.0.0", "3.0.0"];
        // 0.5.0 < 2.0.0 → 2 versions behind.
        assert_eq!(versions_behind("0.5.0", &avail), 2);
    }

    #[test]
    fn empty_available_is_zero() {
        assert_eq!(versions_behind("1.0.0", &[]), 0);
    }

    #[test]
    fn single_element_at_latest_is_zero() {
        assert_eq!(versions_behind("1.0.0", &["1.0.0"]), 0);
    }
}

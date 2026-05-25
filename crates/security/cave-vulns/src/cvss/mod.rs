// SPDX-License-Identifier: AGPL-3.0-or-later
//! CVSS v3.x and v4.0 vector parsing + base-score calculation.
//!
//! Source: DefectDojo/django-DefectDojo@6eab8738 dojo/utils.py::parse_cvss_data
//!         (which delegates to the third-party `cvss` Python lib);
//!         the calculator is a clean-room Rust port of the FIRST
//!         specifications. References:
//!         - CVSS v3.1: <https://www.first.org/cvss/v3.1/specification-document>
//!         - CVSS v4.0: <https://www.first.org/cvss/v4-0/specification-document>

use crate::finding::FindingSeverity;

pub mod v3;
pub mod v4;

/// Auto-promote a finding's severity from a numeric base score.
///
/// Source: DefectDojo/django-DefectDojo@6eab8738
///         dojo/tools/trivy/parser.py::convert_cvss_score (canonical FIRST cuts)
pub fn severity_from_score(score: f32) -> FindingSeverity {
    if score >= 9.0 {
        FindingSeverity::Critical
    } else if score >= 7.0 {
        FindingSeverity::High
    } else if score >= 4.0 {
        FindingSeverity::Medium
    } else if score > 0.0 {
        FindingSeverity::Low
    } else {
        FindingSeverity::Info
    }
}

/// Whichever CVSS version is set on a finding wins. Prefers v4 over v3.
pub fn highest_score(cvssv3_score: Option<f32>, cvssv4_score: Option<f32>) -> Option<f32> {
    match (cvssv3_score, cvssv4_score) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn score_critical_9_or_higher() {
        assert_eq!(severity_from_score(9.0), FindingSeverity::Critical);
        assert_eq!(severity_from_score(10.0), FindingSeverity::Critical);
    }

    #[test]
    fn score_high_7_to_8_99() {
        assert_eq!(severity_from_score(7.0), FindingSeverity::High);
        assert_eq!(severity_from_score(8.9), FindingSeverity::High);
    }

    #[test]
    fn score_medium_4_to_6_99() {
        assert_eq!(severity_from_score(4.0), FindingSeverity::Medium);
        assert_eq!(severity_from_score(6.9), FindingSeverity::Medium);
    }

    #[test]
    fn score_low_just_above_zero() {
        assert_eq!(severity_from_score(0.1), FindingSeverity::Low);
        assert_eq!(severity_from_score(3.9), FindingSeverity::Low);
    }

    #[test]
    fn score_info_at_zero() {
        assert_eq!(severity_from_score(0.0), FindingSeverity::Info);
    }

    #[test]
    fn highest_picks_max() {
        assert_eq!(highest_score(Some(7.5), Some(9.0)), Some(9.0));
        assert_eq!(highest_score(Some(7.5), None), Some(7.5));
        assert_eq!(highest_score(None, Some(9.0)), Some(9.0));
        assert_eq!(highest_score(None, None), None);
    }
}

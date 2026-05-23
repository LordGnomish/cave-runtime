// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Inherited Risk Score — `model/ProjectMetrics#inheritedRiskScore`.
//!
//! Weighted sum used by Dependency-Track defaults (see
//! `ConfigPropertyConstants.SCAN_RISKSCORE_*`):
//!   critical = 10
//!   high     = 5
//!   medium   = 3
//!   low      = 1
//!   unassigned = 5

use crate::models::{Severity, Vulnerability};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RiskWeights {
    pub critical: f64,
    pub high: f64,
    pub medium: f64,
    pub low: f64,
    pub unassigned: f64,
}

impl Default for RiskWeights {
    fn default() -> Self {
        Self {
            critical: 10.0,
            high: 5.0,
            medium: 3.0,
            low: 1.0,
            unassigned: 5.0,
        }
    }
}

pub fn inherited_risk(vulns: &[Vulnerability], w: RiskWeights) -> f64 {
    vulns
        .iter()
        .map(|v| match v.severity {
            Severity::Critical => w.critical,
            Severity::High => w.high,
            Severity::Medium => w.medium,
            Severity::Low => w.low,
            Severity::Info => 0.0,
            Severity::Unassigned => w.unassigned,
        })
        .sum()
}

/// Per-project aggregate when one component → many vulnerabilities.
pub fn project_risk(
    components_with_vulns: &HashMap<Uuid, Vec<Vulnerability>>,
    w: RiskWeights,
) -> f64 {
    components_with_vulns
        .values()
        .map(|vs| inherited_risk(vs, w))
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::VulnSource;

    fn v(s: Severity) -> Vulnerability {
        let mut x = Vulnerability::new("CVE-X", VulnSource::Nvd);
        x.severity = s;
        x
    }

    #[test]
    fn default_weights_sum_correctly() {
        let vs = [
            v(Severity::Critical),
            v(Severity::High),
            v(Severity::Medium),
            v(Severity::Low),
        ];
        let r = inherited_risk(&vs, RiskWeights::default());
        assert!((r - (10.0 + 5.0 + 3.0 + 1.0)).abs() < f64::EPSILON);
    }

    #[test]
    fn unassigned_treated_as_5_by_default() {
        let r = inherited_risk(&[v(Severity::Unassigned)], RiskWeights::default());
        assert!((r - 5.0).abs() < f64::EPSILON);
    }

    #[test]
    fn info_contributes_zero() {
        let r = inherited_risk(&[v(Severity::Info)], RiskWeights::default());
        assert_eq!(r, 0.0);
    }

    #[test]
    fn empty_vulns_zero() {
        let r = inherited_risk(&[], RiskWeights::default());
        assert_eq!(r, 0.0);
    }

    #[test]
    fn project_risk_sums_per_component() {
        let mut m = HashMap::new();
        m.insert(Uuid::new_v4(), vec![v(Severity::Critical)]);
        m.insert(Uuid::new_v4(), vec![v(Severity::High), v(Severity::Low)]);
        let r = project_risk(&m, RiskWeights::default());
        assert!((r - (10.0 + 5.0 + 1.0)).abs() < f64::EPSILON);
    }

    #[test]
    fn custom_weights_override() {
        let w = RiskWeights {
            critical: 100.0,
            high: 50.0,
            medium: 30.0,
            low: 10.0,
            unassigned: 1.0,
        };
        assert_eq!(inherited_risk(&[v(Severity::Critical)], w), 100.0);
    }
}

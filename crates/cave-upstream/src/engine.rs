// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Business logic engine for cave-upstream.

use crate::models::{UpstreamAlert, UpstreamAlertType, UpstreamService};
use chrono::Utc;

/// Returns the number of days until the service reaches end-of-life, or None if not set.
pub fn days_until_eol(service: &UpstreamService) -> Option<i64> {
    service.eol_date.map(|eol| {
        let delta = eol.signed_duration_since(Utc::now());
        delta.num_days()
    })
}

/// Returns the number of days until the service is deprecated, or None if not set.
pub fn days_until_deprecation(service: &UpstreamService) -> Option<i64> {
    service.deprecation_date.map(|dep| {
        let delta = dep.signed_duration_since(Utc::now());
        delta.num_days()
    })
}

/// Generate alerts for a service based on its state.
/// - EolWarning if eol_date is within 90 days
/// - DeprecationWarning if deprecation_date is within 30 days
pub fn generate_alerts(service: &UpstreamService) -> Vec<UpstreamAlert> {
    let mut alerts = Vec::new();
    let now = Utc::now();

    if let Some(days) = days_until_eol(service) {
        if days >= 0 && days <= 90 {
            alerts.push(UpstreamAlert {
                upstream_id: service.id,
                alert_type: UpstreamAlertType::EolWarning,
                message: format!(
                    "Service '{}' reaches end-of-life in {} days",
                    service.name, days
                ),
                severity: if days <= 14 { "critical".to_string() } else { "warning".to_string() },
                created_at: now,
                resolved_at: None,
            });
        }
    }

    if let Some(days) = days_until_deprecation(service) {
        if days >= 0 && days <= 30 {
            alerts.push(UpstreamAlert {
                upstream_id: service.id,
                alert_type: UpstreamAlertType::DeprecationWarning,
                message: format!(
                    "Service '{}' will be deprecated in {} days",
                    service.name, days
                ),
                severity: if days <= 7 { "critical".to_string() } else { "warning".to_string() },
                created_at: now,
                resolved_at: None,
            });
        }
    }

    alerts
}

/// Assess the risk level of a given SPDX license string.
/// Returns "high", "medium", "low", or "unknown".
pub fn license_risk(license: &str) -> &'static str {
    let l = license.to_lowercase();
    // High-risk: copyleft licenses that can affect proprietary usage
    if l.contains("gpl") || l.contains("agpl") || l.contains("sspl") || l.contains("eupl") {
        return "high";
    }
    // Medium-risk: weak copyleft
    if l.contains("lgpl") || l.contains("mpl") || l.contains("cddl") || l.contains("epl") {
        return "medium";
    }
    // Low-risk: permissive
    if l.contains("mit")
        || l.contains("apache")
        || l.contains("bsd")
        || l.contains("isc")
        || l.contains("cc0")
        || l.contains("unlicense")
    {
        return "low";
    }
    "unknown"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_license_risk_gpl() {
        assert_eq!(license_risk("GPL-3.0"), "high");
        assert_eq!(license_risk("AGPL-3.0"), "high");
    }

    #[test]
    fn test_license_risk_permissive() {
        assert_eq!(license_risk("MIT"), "low");
        assert_eq!(license_risk("Apache-2.0"), "low");
        assert_eq!(license_risk("BSD-3-Clause"), "low");
    }

    #[test]
    fn test_license_risk_unknown() {
        assert_eq!(license_risk("Custom-Proprietary"), "unknown");
    }
}

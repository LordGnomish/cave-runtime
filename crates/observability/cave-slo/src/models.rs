// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Core SLO definition — maps to nobl9-go SLO struct.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SLO {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub target_percentage: f64,
    pub window_days: u32,
    pub metric_type: MetricType,
    pub created_at: DateTime<Utc>,
    /// Most-recently evaluated SLI value (0–100).
    #[serde(default)]
    pub current_sli: f64,
    /// Derived status from latest evaluation.
    #[serde(default)]
    pub status: SloStatus,
}

/// What kind of metric an SLO measures.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MetricType {
    Availability,
    Latency,
    ErrorRate,
    Throughput,
}

/// SLI indicator — carries the raw measurement for computing error rate.
/// Mirrors nobl9-go's `Indicator` hierarchy (Ratio/Threshold/Latency).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SloIndicator {
    /// good / total request pair (classic availability ratio)
    Ratio { good: u64, total: u64 },
    /// Latency check: p99_ms vs. configured threshold
    Latency { p99_ms: f64, threshold_ms: f64 },
    /// Numeric threshold comparison (e.g., queue depth, error count)
    Threshold { value: f64, threshold: f64 },
}

impl SloIndicator {
    /// Returns the error rate as a percentage (0–100).
    /// For Ratio: `(total - good) / total * 100`.
    /// For Latency: 0% if p99 < threshold, else 100%.
    /// For Threshold: 0% if value < threshold, else 100%.
    pub fn error_rate_pct(&self) -> f64 {
        match self {
            SloIndicator::Ratio { good, total } => {
                if *total == 0 {
                    return 0.0;
                }
                let bad = total.saturating_sub(*good);
                bad as f64 / *total as f64 * 100.0
            }
            SloIndicator::Latency { p99_ms, threshold_ms } => {
                if p99_ms < threshold_ms { 0.0 } else { 100.0 }
            }
            SloIndicator::Threshold { value, threshold } => {
                if value < threshold { 0.0 } else { 100.0 }
            }
        }
    }
}

/// Named objective within an SLO — supports composite / multi-window SLOs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SloObjective {
    pub name: String,
    /// Target reliability, e.g. 99.9 means 99.9%.
    pub target: f64,
    /// Rolling window length in days.
    pub window_days: u32,
    /// Weight used when compositing multiple objectives (0.0–1.0).
    pub weight: f64,
}

impl SloObjective {
    /// Returns how many minutes of bad events are allowed in the window.
    pub fn allowed_bad_minutes(&self) -> f64 {
        let total_minutes = self.window_days as f64 * 24.0 * 60.0;
        total_minutes * (1.0 - self.target / 100.0)
    }

    /// Returns the total window length in minutes.
    pub fn window_minutes(&self) -> f64 {
        self.window_days as f64 * 24.0 * 60.0
    }
}

/// Evaluated status of an SLO at a point in time.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SloStatus {
    #[default]
    Unknown,
    /// Burn rate < 2.0 — well within budget
    Ok,
    /// Burn rate 2.0–5.0 — slow budget drain
    AtRisk,
    /// Burn rate 5.0–14.4 — fast drain, will breach if continued
    Breaching,
    /// Burn rate ≥ 14.4 — immediate budget exhaustion
    Breached,
}

impl SloStatus {
    /// Derive status from short-window burn rate (the 1-hour window is the
    /// primary signal per Google SRE book page-worthy burn rates).
    pub fn from_burn_rate(burn_rate: f64) -> Self {
        if burn_rate >= 14.4 {
            SloStatus::Breached
        } else if burn_rate >= 5.0 {
            SloStatus::Breaching
        } else if burn_rate >= 2.0 {
            SloStatus::AtRisk
        } else {
            SloStatus::Ok
        }
    }
}

/// Computed error budget for an SLO evaluation window.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ErrorBudget {
    pub slo_id: Uuid,
    pub total_minutes: f64,
    pub allowed_bad_minutes: f64,
    pub consumed_bad_minutes: f64,
    pub remaining_minutes: f64,
    pub remaining_percentage: f64,
    pub is_breached: bool,
}

/// Multi-window burn rate alert result.
/// Matches the Google SRE multi-window approach (1h + 5m alerting).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BurnRateAlert {
    pub slo_id: Uuid,
    pub window_hours: u32,
    pub burn_rate: f64,
    pub threshold: f64,
    pub is_firing: bool,
}

/// Key/value annotation bag for SLOs.
/// Matches nobl9-go `Metadata.Annotations` map.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct SloAnnotations(pub HashMap<String, String>);

impl SloAnnotations {
    pub fn set(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.0.insert(key.into(), value.into());
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(String::as_str)
    }

    pub fn remove(&mut self, key: &str) {
        self.0.remove(key);
    }
}

/// Aggregate statistics across all SLOs in the store.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SloStats {
    pub total: u64,
    pub ok: u64,
    pub at_risk: u64,
    pub breaching: u64,
    pub breached: u64,
    /// Mean current_sli across all SLOs.
    pub avg_compliance: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_slo() -> SLO {
        SLO {
            id: Uuid::new_v4(),
            name: "api-availability".to_string(),
            description: "API must be up 99.9% of the time".to_string(),
            target_percentage: 99.9,
            window_days: 30,
            metric_type: MetricType::Availability,
            created_at: Utc::now(),
            current_sli: 99.95,
            status: SloStatus::Ok,
        }
    }

    #[test]
    fn test_slo_serde_roundtrip() {
        let slo = make_slo();
        let json = serde_json::to_string(&slo).unwrap();
        let restored: SLO = serde_json::from_str(&json).unwrap();
        assert_eq!(slo, restored);
    }

    #[test]
    fn test_metric_type_serde() {
        for (variant, expected) in [
            (MetricType::Availability, "\"availability\""),
            (MetricType::Latency, "\"latency\""),
            (MetricType::ErrorRate, "\"error_rate\""),
            (MetricType::Throughput, "\"throughput\""),
        ] {
            let s = serde_json::to_string(&variant).unwrap();
            assert_eq!(s, expected);
            let restored: MetricType = serde_json::from_str(&s).unwrap();
            assert_eq!(restored, variant);
        }
    }

    #[test]
    fn test_error_budget_serde_roundtrip() {
        let slo_id = Uuid::new_v4();
        let budget = ErrorBudget {
            slo_id,
            total_minutes: 43200.0,
            allowed_bad_minutes: 43.2,
            consumed_bad_minutes: 20.0,
            remaining_minutes: 23.2,
            remaining_percentage: 53.7,
            is_breached: false,
        };
        let json = serde_json::to_string(&budget).unwrap();
        let restored: ErrorBudget = serde_json::from_str(&json).unwrap();
        assert_eq!(budget, restored);
    }

    #[test]
    fn test_burn_rate_alert_serde_roundtrip() {
        let alert = BurnRateAlert {
            slo_id: Uuid::new_v4(),
            window_hours: 1,
            burn_rate: 14.4,
            threshold: 14.4,
            is_firing: true,
        };
        let json = serde_json::to_string(&alert).unwrap();
        let restored: BurnRateAlert = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.slo_id, alert.slo_id);
        assert_eq!(restored.is_firing, alert.is_firing);
        assert!((restored.burn_rate - alert.burn_rate).abs() < 1e-9);
    }

    #[test]
    fn test_slo_target_preserved() {
        let slo = make_slo();
        let json = serde_json::to_string(&slo).unwrap();
        let restored: SLO = serde_json::from_str(&json).unwrap();
        assert!((restored.target_percentage - 99.9).abs() < 1e-9);
        assert_eq!(restored.window_days, 30);
    }

    #[test]
    fn test_slo_status_ordering() {
        assert_eq!(SloStatus::from_burn_rate(0.0), SloStatus::Ok);
        assert_eq!(SloStatus::from_burn_rate(1.99), SloStatus::Ok);
        assert_eq!(SloStatus::from_burn_rate(2.0), SloStatus::AtRisk);
        assert_eq!(SloStatus::from_burn_rate(5.0), SloStatus::Breaching);
        assert_eq!(SloStatus::from_burn_rate(14.4), SloStatus::Breached);
    }
}

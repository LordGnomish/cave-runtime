// SPDX-License-Identifier: AGPL-3.0-or-later
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SLO {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub target_percentage: f64,
    pub window_days: u32,
    pub metric_type: MetricType,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MetricType {
    Availability,
    Latency,
    ErrorRate,
    Throughput,
}

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BurnRateAlert {
    pub slo_id: Uuid,
    pub window_hours: u32,
    pub burn_rate: f64,
    pub threshold: f64,
    pub is_firing: bool,
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
}

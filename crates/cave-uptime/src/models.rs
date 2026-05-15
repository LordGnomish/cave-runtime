// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UptimeProbe {
    pub id: Uuid,
    pub name: String,
    pub target_url: String,
    pub probe_type: ProbeType,
    pub interval_seconds: u32,
    pub timeout_ms: u32,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ProbeType {
    Http,
    Tcp,
    Ping,
    Dns,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProbeResult {
    pub probe_id: Uuid,
    pub success: bool,
    pub latency_ms: u64,
    pub status_code: Option<u16>,
    pub error: Option<String>,
    pub checked_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UptimeStats {
    pub probe_id: Uuid,
    pub uptime_percentage: f64,
    pub avg_latency_ms: f64,
    pub total_checks: u64,
    pub successful_checks: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_probe() -> UptimeProbe {
        UptimeProbe {
            id: Uuid::new_v4(),
            name: "API Health".to_string(),
            target_url: "https://api.example.com/health".to_string(),
            probe_type: ProbeType::Http,
            interval_seconds: 60,
            timeout_ms: 5000,
            enabled: true,
        }
    }

    fn make_result(probe_id: Uuid, success: bool, latency_ms: u64) -> ProbeResult {
        ProbeResult {
            probe_id,
            success,
            latency_ms,
            status_code: if success { Some(200) } else { None },
            error: if success { None } else { Some("connection refused".to_string()) },
            checked_at: Utc::now(),
        }
    }

    #[test]
    fn test_uptime_probe_roundtrip() {
        let probe = make_probe();
        let json = serde_json::to_string(&probe).unwrap();
        let decoded: UptimeProbe = serde_json::from_str(&json).unwrap();
        assert_eq!(probe, decoded);
    }

    #[test]
    fn test_probe_type_serde_names() {
        let pt = ProbeType::Dns;
        let json = serde_json::to_string(&pt).unwrap();
        assert_eq!(json, "\"dns\"");
        let decoded: ProbeType = serde_json::from_str(&json).unwrap();
        assert_eq!(pt, decoded);
    }

    #[test]
    fn test_probe_result_success_roundtrip() {
        let id = Uuid::new_v4();
        let result = make_result(id, true, 42);
        let json = serde_json::to_string(&result).unwrap();
        let decoded: ProbeResult = serde_json::from_str(&json).unwrap();
        assert_eq!(result, decoded);
    }

    #[test]
    fn test_probe_result_failure_roundtrip() {
        let id = Uuid::new_v4();
        let result = make_result(id, false, 0);
        let json = serde_json::to_string(&result).unwrap();
        let decoded: ProbeResult = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.success, false);
        assert!(decoded.error.is_some());
    }

    #[test]
    fn test_uptime_stats_roundtrip() {
        let stats = UptimeStats {
            probe_id: Uuid::new_v4(),
            uptime_percentage: 99.5,
            avg_latency_ms: 123.4,
            total_checks: 1000,
            successful_checks: 995,
        };
        let json = serde_json::to_string(&stats).unwrap();
        let decoded: UptimeStats = serde_json::from_str(&json).unwrap();
        assert_eq!(stats, decoded);
    }
}

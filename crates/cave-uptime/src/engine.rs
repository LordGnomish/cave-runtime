use crate::models::{ProbeResult, UptimeStats};
use uuid::Uuid;

/// Calculate uptime stats from a list of probe results
pub fn calculate_stats(probe_id: Uuid, results: &[ProbeResult]) -> UptimeStats {
    let total = results.len() as u64;
    if total == 0 {
        return UptimeStats {
            probe_id,
            uptime_percentage: 100.0,
            avg_latency_ms: 0.0,
            total_checks: 0,
            successful_checks: 0,
        };
    }
    let successful = results.iter().filter(|r| r.success).count() as u64;
    let avg_latency = if successful == 0 {
        0.0
    } else {
        results
            .iter()
            .filter(|r| r.success)
            .map(|r| r.latency_ms as f64)
            .sum::<f64>()
            / successful as f64
    };
    UptimeStats {
        probe_id,
        uptime_percentage: successful as f64 / total as f64 * 100.0,
        avg_latency_ms: avg_latency,
        total_checks: total,
        successful_checks: successful,
    }
}

/// Filter results to only failed ones
pub fn failed_results(results: &[ProbeResult]) -> Vec<&ProbeResult> {
    results.iter().filter(|r| !r.success).collect()
}

/// Check if uptime is below threshold percentage
pub fn is_below_threshold(stats: &UptimeStats, threshold: f64) -> bool {
    stats.uptime_percentage < threshold
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn make_result(success: bool, latency_ms: u64) -> ProbeResult {
        ProbeResult {
            probe_id: Uuid::new_v4(),
            success,
            latency_ms,
            status_code: if success { Some(200) } else { None },
            error: if success { None } else { Some("timeout".to_string()) },
            checked_at: Utc::now(),
        }
    }

    #[test]
    fn test_calculate_stats_all_success() {
        let id = Uuid::new_v4();
        let results = vec![
            make_result(true, 100),
            make_result(true, 200),
            make_result(true, 150),
        ];
        let stats = calculate_stats(id, &results);
        assert_eq!(stats.uptime_percentage, 100.0);
        assert_eq!(stats.total_checks, 3);
        assert_eq!(stats.successful_checks, 3);
        assert!((stats.avg_latency_ms - 150.0).abs() < 0.01);
    }

    #[test]
    fn test_calculate_stats_all_fail() {
        let id = Uuid::new_v4();
        let results = vec![make_result(false, 0), make_result(false, 0)];
        let stats = calculate_stats(id, &results);
        assert_eq!(stats.uptime_percentage, 0.0);
        assert_eq!(stats.successful_checks, 0);
    }

    #[test]
    fn test_calculate_stats_empty() {
        let id = Uuid::new_v4();
        let stats = calculate_stats(id, &[]);
        assert_eq!(stats.uptime_percentage, 100.0);
        assert_eq!(stats.total_checks, 0);
    }

    #[test]
    fn test_calculate_stats_mixed() {
        let id = Uuid::new_v4();
        let results = vec![
            make_result(true, 100),
            make_result(true, 200),
            make_result(true, 300),
            make_result(false, 0),
        ];
        let stats = calculate_stats(id, &results);
        assert!((stats.uptime_percentage - 75.0).abs() < 0.01);
        assert_eq!(stats.total_checks, 4);
        assert_eq!(stats.successful_checks, 3);
    }

    #[test]
    fn test_failed_results_filter() {
        let results = vec![
            make_result(true, 100),
            make_result(false, 0),
            make_result(true, 200),
            make_result(false, 0),
        ];
        let failed = failed_results(&results);
        assert_eq!(failed.len(), 2);
        assert!(failed.iter().all(|r| !r.success));
    }

    #[test]
    fn test_is_below_threshold() {
        let id = Uuid::new_v4();
        let stats = UptimeStats {
            probe_id: id,
            uptime_percentage: 95.0,
            avg_latency_ms: 100.0,
            total_checks: 100,
            successful_checks: 95,
        };
        assert!(is_below_threshold(&stats, 99.0));
        assert!(!is_below_threshold(&stats, 90.0));
    }
}

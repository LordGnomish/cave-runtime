// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use crate::models::{DeployStatus, DeploymentRecord};

pub fn deployment_frequency(records: &[DeploymentRecord], days: u32) -> f64 {
    let successful = records
        .iter()
        .filter(|r| r.status == DeployStatus::Success)
        .count();
    successful as f64 / days as f64
}

pub fn change_failure_rate(records: &[DeploymentRecord]) -> f64 {
    if records.is_empty() {
        return 0.0;
    }
    let failures = records
        .iter()
        .filter(|r| r.status == DeployStatus::Failure)
        .count();
    failures as f64 / records.len() as f64
}

pub fn successful_deployments(records: &[DeploymentRecord]) -> Vec<&DeploymentRecord> {
    records
        .iter()
        .filter(|r| r.status == DeployStatus::Success)
        .collect()
}

pub fn average_duration_secs(records: &[DeploymentRecord]) -> Option<f64> {
    let completed: Vec<f64> = records
        .iter()
        .filter_map(|r| {
            r.finished_at
                .map(|end| (end - r.started_at).num_seconds() as f64)
        })
        .collect();
    if completed.is_empty() {
        None
    } else {
        Some(completed.iter().sum::<f64>() / completed.len() as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{DeployStatus, DeploymentRecord};
    use chrono::{Duration, Utc};
    use uuid::Uuid;

    fn make_record(status: DeployStatus, duration_secs: Option<i64>) -> DeploymentRecord {
        let started_at = Utc::now();
        let finished_at = duration_secs.map(|d| started_at + Duration::seconds(d));
        DeploymentRecord {
            id: Uuid::new_v4(),
            pipeline: "main".to_string(),
            environment: "prod".to_string(),
            status,
            started_at,
            finished_at,
            commit_sha: "abc123".to_string(),
        }
    }

    #[test]
    fn test_deployment_frequency() {
        let records = vec![
            make_record(DeployStatus::Success, Some(120)),
            make_record(DeployStatus::Success, Some(90)),
            make_record(DeployStatus::Success, Some(100)),
            make_record(DeployStatus::Success, Some(80)),
            make_record(DeployStatus::Success, Some(70)),
            make_record(DeployStatus::Failure, Some(30)),
        ];
        let freq = deployment_frequency(&records, 5);
        assert!((freq - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_change_failure_rate() {
        let records = vec![
            make_record(DeployStatus::Success, Some(100)),
            make_record(DeployStatus::Success, Some(100)),
            make_record(DeployStatus::Success, Some(100)),
            make_record(DeployStatus::Failure, Some(30)),
        ];
        let rate = change_failure_rate(&records);
        assert!((rate - 0.25).abs() < 0.001);
    }

    #[test]
    fn test_change_failure_rate_empty() {
        let records: Vec<DeploymentRecord> = vec![];
        assert_eq!(change_failure_rate(&records), 0.0);
    }

    #[test]
    fn test_successful_deployments_filter() {
        let records = vec![
            make_record(DeployStatus::Success, Some(100)),
            make_record(DeployStatus::Failure, Some(30)),
            make_record(DeployStatus::Aborted, None),
            make_record(DeployStatus::Success, Some(90)),
        ];
        let successful = successful_deployments(&records);
        assert_eq!(successful.len(), 2);
        for r in &successful {
            assert_eq!(r.status, DeployStatus::Success);
        }
    }

    #[test]
    fn test_average_duration_secs_none_for_pending() {
        let records = vec![
            make_record(DeployStatus::Success, None),
            make_record(DeployStatus::Failure, None),
        ];
        assert_eq!(average_duration_secs(&records), None);
    }

    #[test]
    fn test_average_duration_secs_calculated() {
        let records = vec![
            make_record(DeployStatus::Success, Some(100)),
            make_record(DeployStatus::Success, Some(200)),
        ];
        let avg = average_duration_secs(&records).unwrap();
        assert!((avg - 150.0).abs() < 1.0);
    }
}

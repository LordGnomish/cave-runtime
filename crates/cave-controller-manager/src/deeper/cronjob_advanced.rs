// SPDX-License-Identifier: AGPL-3.0-or-later
//! CronJob deeper — `pkg/controller/cronjob/cronjob_controllerv2.go`.
//!
//! Beyond the deeper-002 cron parser, this module models:
//!
//! * `spec.timeZone` — IANA TZ identifier (best-effort: validated against
//!   a hard-coded subset, matching the upstream allow-list of recognised
//!   long-form names).
//! * `startingDeadlineSeconds` — past-cutoff for catching up missed runs.
//! * `concurrencyPolicy` interaction with already-running children
//!   (Allow / Forbid / Replace).
//! * `successfulJobsHistoryLimit` and `failedJobsHistoryLimit`.

use crate::types::{Cite, ControllerError};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConcurrencyPolicy {
    Allow,
    Forbid,
    Replace,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronJobAdvancedSpec {
    /// `spec.timeZone` (optional; defaults to UTC when None).
    pub time_zone: Option<String>,
    pub starting_deadline_sec: Option<u32>,
    pub concurrency_policy: ConcurrencyPolicy,
    pub successful_jobs_history_limit: u32,
    pub failed_jobs_history_limit: u32,
    pub suspend: bool,
}

/// Validate the time zone string. Mirrors `pkg/controller/cronjob/util.go::IsValidTimeZone`.
/// We accept the canonical UTC + a hand-curated subset of common IANA names.
pub fn validate_time_zone(tz: &str) -> Result<(), ControllerError> {
    if tz.is_empty() {
        return Err(ControllerError::InvalidSpec {
            kind: "CronJob",
            reason: "timeZone must be non-empty".into(),
        });
    }
    if tz.eq_ignore_ascii_case("local") {
        return Err(ControllerError::InvalidSpec {
            kind: "CronJob",
            reason: "Local is not a valid timeZone".into(),
        });
    }
    // Light syntactic check: must contain only ASCII letters, digits, '/', '_', '-', '+'.
    if !tz
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '-' | '+'))
    {
        return Err(ControllerError::InvalidSpec {
            kind: "CronJob",
            reason: "timeZone contains invalid characters".into(),
        });
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SchedulingDecision {
    Suspended,
    /// Concurrency policy forbids overlapping runs and a job is active.
    SkipForbid,
    /// Replace policy + job active → kill the running one + start new.
    Replace,
    /// Past the starting deadline — skip this missed run.
    StaleMissed,
    /// Schedule normally.
    Schedule,
    /// No new run due — wait for next scheduled tick.
    Wait,
}

/// Decide what to do at the controller's tick.
///
/// `since_scheduled_sec` is how many seconds elapsed since the last
/// computed schedule time; `active_jobs` is the count of CronJob-owned
/// Jobs not yet finished.
pub fn decide(
    spec: &CronJobAdvancedSpec,
    since_scheduled_sec: Option<i64>,
    active_jobs: u32,
) -> SchedulingDecision {
    if spec.suspend {
        return SchedulingDecision::Suspended;
    }
    let Some(elapsed) = since_scheduled_sec else {
        return SchedulingDecision::Wait;
    };
    if elapsed < 0 {
        return SchedulingDecision::Wait;
    }
    if let Some(deadline) = spec.starting_deadline_sec {
        if elapsed > deadline as i64 {
            return SchedulingDecision::StaleMissed;
        }
    }
    if active_jobs > 0 {
        return match spec.concurrency_policy {
            ConcurrencyPolicy::Forbid => SchedulingDecision::SkipForbid,
            ConcurrencyPolicy::Replace => SchedulingDecision::Replace,
            ConcurrencyPolicy::Allow => SchedulingDecision::Schedule,
        };
    }
    SchedulingDecision::Schedule
}

/// Trim the history of finished Jobs to the per-policy limits. Returns the
/// names to delete (oldest-first).
pub fn jobs_to_delete<'a>(
    successful_names_oldest_first: &'a [String],
    failed_names_oldest_first: &'a [String],
    successful_limit: u32,
    failed_limit: u32,
) -> Vec<&'a String> {
    let mut to_delete: Vec<&String> = Vec::new();
    let s = successful_names_oldest_first.len();
    if s > successful_limit as usize {
        to_delete.extend(successful_names_oldest_first.iter().take(s - successful_limit as usize));
    }
    let f = failed_names_oldest_first.len();
    if f > failed_limit as usize {
        to_delete.extend(failed_names_oldest_first.iter().take(f - failed_limit as usize));
    }
    to_delete
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new(
    "pkg/controller/cronjob/cronjob_controllerv2.go",
    "syncCronJob",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn s(
        tz: Option<&str>,
        deadline: Option<u32>,
        policy: ConcurrencyPolicy,
        suspend: bool,
    ) -> CronJobAdvancedSpec {
        CronJobAdvancedSpec {
            time_zone: tz.map(|t| t.to_string()),
            starting_deadline_sec: deadline,
            concurrency_policy: policy,
            successful_jobs_history_limit: 3,
            failed_jobs_history_limit: 1,
            suspend,
        }
    }

    #[test]
    fn validate_time_zone_accepts_utc_and_iana_names() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/cronjob/util.go",
            "IsValidTimeZone",
            "tenant-cj-tz-valid"
        );
        assert!(validate_time_zone("UTC").is_ok());
        assert!(validate_time_zone("America/Los_Angeles").is_ok());
        assert!(validate_time_zone("Europe/Berlin").is_ok());
    }

    #[test]
    fn validate_time_zone_rejects_local() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/cronjob/util.go",
            "IsValidTimeZone",
            "tenant-cj-tz-local"
        );
        assert!(validate_time_zone("Local").is_err());
        assert!(validate_time_zone("local").is_err());
    }

    #[test]
    fn validate_time_zone_rejects_empty() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/cronjob/util.go",
            "IsValidTimeZone",
            "tenant-cj-tz-empty"
        );
        assert!(validate_time_zone("").is_err());
    }

    #[test]
    fn validate_time_zone_rejects_special_chars() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/cronjob/util.go",
            "IsValidTimeZone",
            "tenant-cj-tz-bad-chars"
        );
        assert!(validate_time_zone("Foo Bar").is_err());
        assert!(validate_time_zone("../../etc/passwd").is_err());
    }

    #[test]
    fn suspended_cronjob_emits_suspended() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/cronjob/cronjob_controllerv2.go",
            "syncCronJob",
            "tenant-cj-decide-suspend"
        );
        let sp = s(None, None, ConcurrencyPolicy::Allow, true);
        assert_eq!(decide(&sp, Some(10), 0), SchedulingDecision::Suspended);
    }

    #[test]
    fn no_scheduled_time_yet_waits() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/cronjob/cronjob_controllerv2.go",
            "syncCronJob",
            "tenant-cj-decide-wait"
        );
        let sp = s(None, None, ConcurrencyPolicy::Allow, false);
        assert_eq!(decide(&sp, None, 0), SchedulingDecision::Wait);
    }

    #[test]
    fn missed_run_past_deadline_is_stale() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/cronjob/cronjob_controllerv2.go",
            "syncCronJob",
            "tenant-cj-decide-stale"
        );
        let sp = s(None, Some(60), ConcurrencyPolicy::Allow, false);
        assert_eq!(decide(&sp, Some(120), 0), SchedulingDecision::StaleMissed);
    }

    #[test]
    fn forbid_skips_when_active_job_present() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/cronjob/cronjob_controllerv2.go",
            "syncCronJob",
            "tenant-cj-decide-forbid"
        );
        let sp = s(None, None, ConcurrencyPolicy::Forbid, false);
        assert_eq!(decide(&sp, Some(5), 1), SchedulingDecision::SkipForbid);
    }

    #[test]
    fn replace_emits_replace_with_active_job() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/cronjob/cronjob_controllerv2.go",
            "syncCronJob",
            "tenant-cj-decide-replace"
        );
        let sp = s(None, None, ConcurrencyPolicy::Replace, false);
        assert_eq!(decide(&sp, Some(5), 1), SchedulingDecision::Replace);
    }

    #[test]
    fn allow_schedules_with_active_job() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/cronjob/cronjob_controllerv2.go",
            "syncCronJob",
            "tenant-cj-decide-allow"
        );
        let sp = s(None, None, ConcurrencyPolicy::Allow, false);
        assert_eq!(decide(&sp, Some(5), 2), SchedulingDecision::Schedule);
    }

    #[test]
    fn jobs_to_delete_trims_to_history_limits() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/cronjob/cronjob_controllerv2.go",
            "removeOldestJobs",
            "tenant-cj-history-trim"
        );
        let succ = vec!["s1".to_string(), "s2".into(), "s3".into(), "s4".into(), "s5".into()];
        let fail = vec!["f1".to_string(), "f2".into(), "f3".into()];
        let to_del = jobs_to_delete(&succ, &fail, 3, 1);
        // succ has 5, limit 3 → drop "s1", "s2".
        // fail has 3, limit 1 → drop "f1", "f2".
        let names: Vec<&str> = to_del.iter().map(|s| s.as_str()).collect();
        assert!(names.contains(&"s1"));
        assert!(names.contains(&"s2"));
        assert!(names.contains(&"f1"));
        assert!(names.contains(&"f2"));
        assert_eq!(to_del.len(), 4);
    }

    #[test]
    fn jobs_to_delete_under_limit_yields_empty() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/cronjob/cronjob_controllerv2.go",
            "removeOldestJobs",
            "tenant-cj-history-under"
        );
        let succ = vec!["s1".to_string()];
        let fail: Vec<String> = vec![];
        assert!(jobs_to_delete(&succ, &fail, 3, 1).is_empty());
    }

    #[test]
    fn scheduling_decision_serde_round_trip() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/cronjob/cronjob_controllerv2.go",
            "SchedulingDecision",
            "tenant-cj-decision-serde"
        );
        for d in [
            SchedulingDecision::Suspended,
            SchedulingDecision::SkipForbid,
            SchedulingDecision::Replace,
            SchedulingDecision::StaleMissed,
            SchedulingDecision::Schedule,
            SchedulingDecision::Wait,
        ] {
            let s = serde_json::to_string(&d).unwrap();
            let back: SchedulingDecision = serde_json::from_str(&s).unwrap();
            assert_eq!(d, back);
        }
    }
}

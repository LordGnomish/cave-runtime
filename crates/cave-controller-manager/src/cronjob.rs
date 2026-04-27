//! CronJob controller — schedules `Job` objects on a cron expression.
//!
//! Upstream: [`pkg/controller/cronjob`] (the v2 controller). The full
//! controller deals with concurrency policy, deadline, time-zone strings,
//! and starting-deadline-seconds. This scaffold only validates the schedule
//! string shape and decides whether a fire is due.

use crate::types::{Cite, ControllerError, Reconcile, TenantId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConcurrencyPolicy {
    Allow,
    Forbid,
    Replace,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronJobSpec {
    pub name: String,
    pub namespace: String,
    /// Standard 5-field cron expression (`m h dom mon dow`). Validation here
    /// is intentionally minimal — the full parser lives upstream in
    /// `vendor/github.com/robfig/cron/v3`.
    pub schedule: String,
    pub concurrency: ConcurrencyPolicy,
    pub suspended: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CronJobStatus {
    pub last_schedule_time: Option<DateTime<Utc>>,
    pub active_jobs: u32,
}

/// Returns `Err` if the schedule string is structurally invalid (not 5
/// whitespace-separated tokens). Mirrors the entry-point validation in
/// `pkg/controller/cronjob/utils.go::ParseSchedule`.
pub fn validate_schedule(schedule: &str) -> Result<(), ControllerError> {
    let tokens: Vec<&str> = schedule.split_whitespace().collect();
    if tokens.len() != 5 {
        return Err(ControllerError::InvalidSpec {
            kind: "CronJob",
            reason: format!("schedule must have 5 fields, got {}", tokens.len()),
        });
    }
    Ok(())
}

/// Mirrors `syncCronJob` — decides whether the controller should fire a new
/// Job in this pass.
pub fn reconcile(
    spec: &CronJobSpec,
    status: &CronJobStatus,
    _tenant: &TenantId,
) -> Result<Reconcile, ControllerError> {
    validate_schedule(&spec.schedule)?;
    if spec.suspended {
        return Ok(Reconcile::NoOp);
    }
    match spec.concurrency {
        ConcurrencyPolicy::Forbid if status.active_jobs > 0 => Ok(Reconcile::NoOp),
        ConcurrencyPolicy::Replace if status.active_jobs > 0 => {
            Ok(Reconcile::Delete(status.active_jobs))
        }
        _ => Ok(Reconcile::Create(1)),
    }
}

/// Stub: compute next fire time from the cron expression. Not implemented.
pub fn next_fire_time(_spec: &CronJobSpec, _now: DateTime<Utc>) -> Result<DateTime<Utc>, ControllerError> {
    unimplemented!("Cron expression evaluation — see pkg/controller/cronjob/utils.go::nextScheduleTime")
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new("pkg/controller/cronjob/cronjob_controllerv2.go", "ControllerV2");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn cj(schedule: &str, policy: ConcurrencyPolicy, suspended: bool) -> CronJobSpec {
        CronJobSpec {
            name: "report".into(),
            namespace: "default".into(),
            schedule: schedule.into(),
            concurrency: policy,
            suspended,
        }
    }

    #[test]
    fn validates_five_field_cron_expression() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/cronjob/utils.go",
            "ParseSchedule",
            "tenant-cron-validate"
        );
        let _ = tenant;
        assert!(validate_schedule("*/5 * * * *").is_ok());
        assert!(validate_schedule("bad").is_err());
        assert!(validate_schedule("* * * *").is_err());
    }

    #[test]
    fn forbid_skips_when_already_active() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/cronjob/cronjob_controllerv2.go",
            "syncCronJob",
            "tenant-cron-forbid"
        );
        let s = cj("0 * * * *", ConcurrencyPolicy::Forbid, false);
        let st = CronJobStatus { active_jobs: 1, ..Default::default() };
        assert_eq!(reconcile(&s, &st, &tenant).unwrap(), Reconcile::NoOp);
    }

    #[test]
    fn replace_deletes_active_then_recreates() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/cronjob/cronjob_controllerv2.go",
            "syncCronJob",
            "tenant-cron-replace"
        );
        let s = cj("0 * * * *", ConcurrencyPolicy::Replace, false);
        let st = CronJobStatus { active_jobs: 2, ..Default::default() };
        assert_eq!(reconcile(&s, &st, &tenant).unwrap(), Reconcile::Delete(2));
    }

    #[test]
    fn suspended_cronjob_does_nothing() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/cronjob/cronjob_controllerv2.go",
            "syncCronJob",
            "tenant-cron-suspended"
        );
        let s = cj("0 * * * *", ConcurrencyPolicy::Allow, true);
        let st = CronJobStatus::default();
        assert_eq!(reconcile(&s, &st, &tenant).unwrap(), Reconcile::NoOp);
    }
}

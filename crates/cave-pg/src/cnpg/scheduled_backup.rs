//! `ScheduledBackup` CRD — cron-driven Backup factory.

use chrono::{DateTime, Duration, Utc};

use super::backup::{Backup, BackupMethod, BackupSpec};

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ScheduledBackupError {
    #[error("invalid cron: {0}")]
    InvalidCron(String),
    #[error("suspended")]
    Suspended,
}

#[derive(Debug, Clone)]
pub struct ScheduledBackupSpec {
    pub cluster_name: String,
    pub schedule: String,
    pub immediate: bool,
    pub suspended: bool,
    pub backup_method: BackupMethod,
    pub backup_owner_reference: bool,
    pub destination_path: String,
    pub retention_days: i32,
}

impl Default for ScheduledBackupSpec {
    fn default() -> Self {
        Self {
            cluster_name: "cluster".into(),
            schedule: "0 0 * * *".into(), // daily midnight (5-field cron)
            immediate: false,
            suspended: false,
            backup_method: BackupMethod::BarmanObjectStore,
            backup_owner_reference: true,
            destination_path: "s3://backups".into(),
            retention_days: 30,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ScheduledBackup {
    pub name: String,
    pub spec: ScheduledBackupSpec,
    pub last_run: Option<DateTime<Utc>>,
    pub next_run: Option<DateTime<Utc>>,
    pub run_count: u64,
}

impl ScheduledBackup {
    pub fn new(name: &str, spec: ScheduledBackupSpec) -> Self {
        Self {
            name: name.into(),
            spec,
            last_run: None,
            next_run: None,
            run_count: 0,
        }
    }

    /// Pre-flight: parse the cron spec — minimal 5-field validation
    /// (`min hour dom mon dow`).
    pub fn validate_cron(&self) -> Result<(), ScheduledBackupError> {
        let parts: Vec<&str> = self.spec.schedule.split_whitespace().collect();
        if parts.len() != 5 {
            return Err(ScheduledBackupError::InvalidCron(format!(
                "expected 5 fields, got {}",
                parts.len()
            )));
        }
        for p in parts {
            if !p.chars().all(|c| {
                c.is_ascii_digit() || c == '*' || c == '/' || c == ',' || c == '-'
            }) {
                return Err(ScheduledBackupError::InvalidCron(format!(
                    "invalid char in field {}",
                    p
                )));
            }
        }
        Ok(())
    }

    /// Compute the next-run timestamp by adding 24h (placeholder for full
    /// cron evaluator). Tested separately by [`compute_next_run_after`].
    pub fn compute_next(&self, now: DateTime<Utc>) -> DateTime<Utc> {
        compute_next_run_after(&self.spec.schedule, now)
    }

    /// Trigger a Backup if the cron is due.
    pub fn maybe_trigger(
        &mut self,
        now: DateTime<Utc>,
        run_id: &str,
    ) -> Result<Option<Backup>, ScheduledBackupError> {
        if self.spec.suspended {
            return Err(ScheduledBackupError::Suspended);
        }
        self.validate_cron()?;
        let due = match self.next_run {
            Some(n) => now >= n,
            None => self.spec.immediate || now >= self.compute_next(now - Duration::days(1)),
        };
        if !due {
            return Ok(None);
        }
        let backup = Backup::new(
            run_id,
            BackupSpec {
                cluster_name: self.spec.cluster_name.clone(),
                method: self.spec.backup_method,
                destination_path: self.spec.destination_path.clone(),
                online: true,
                immediate_checkpoint: true,
                wait_for_archive: true,
                retention_days: self.spec.retention_days,
            },
        );
        self.last_run = Some(now);
        self.next_run = Some(self.compute_next(now));
        self.run_count += 1;
        Ok(Some(backup))
    }
}

/// Bare-bones cron-next: handles `0 0 * * *` (midnight daily) and
/// `*/N * * * *` (every N minutes). Anything else falls back to "+24h".
pub fn compute_next_run_after(schedule: &str, now: DateTime<Utc>) -> DateTime<Utc> {
    let parts: Vec<&str> = schedule.split_whitespace().collect();
    if parts.len() != 5 {
        return now + Duration::hours(24);
    }
    if let Some(stripped) = parts[0].strip_prefix("*/") {
        if let Ok(n) = stripped.parse::<i64>() {
            return now + Duration::minutes(n);
        }
    }
    if parts == ["0", "0", "*", "*", "*"] {
        let mut next = now
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            + Duration::days(1);
        if next <= now {
            next = next + Duration::days(1);
        }
        return next;
    }
    now + Duration::hours(24)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_cron_accepts_five_fields() {
        let s = ScheduledBackup::new("s", ScheduledBackupSpec::default());
        assert!(s.validate_cron().is_ok());
    }

    #[test]
    fn validate_cron_rejects_six_fields() {
        let mut s = ScheduledBackup::new("s", ScheduledBackupSpec::default());
        s.spec.schedule = "0 0 * * * *".into();
        assert!(s.validate_cron().is_err());
    }

    #[test]
    fn validate_cron_rejects_letter() {
        let mut s = ScheduledBackup::new("s", ScheduledBackupSpec::default());
        s.spec.schedule = "0 0 * * MON".into();
        assert!(s.validate_cron().is_err());
    }

    #[test]
    fn validate_cron_accepts_step_syntax() {
        let mut s = ScheduledBackup::new("s", ScheduledBackupSpec::default());
        s.spec.schedule = "*/15 * * * *".into();
        assert!(s.validate_cron().is_ok());
    }

    #[test]
    fn suspended_returns_error_on_trigger() {
        let mut s = ScheduledBackup::new("s", ScheduledBackupSpec::default());
        s.spec.suspended = true;
        let err = s.maybe_trigger(Utc::now(), "run").unwrap_err();
        assert_eq!(err, ScheduledBackupError::Suspended);
    }

    #[test]
    fn trigger_increments_run_count() {
        let mut s = ScheduledBackup::new("s", ScheduledBackupSpec::default());
        s.spec.immediate = true;
        let _ = s.maybe_trigger(Utc::now(), "run-1").unwrap();
        assert!(s.run_count >= 1);
    }

    #[test]
    fn compute_next_step_minutes() {
        let now = Utc::now();
        let next = compute_next_run_after("*/5 * * * *", now);
        assert_eq!(next, now + Duration::minutes(5));
    }

    #[test]
    fn compute_next_daily_midnight_advances() {
        let now = Utc::now();
        let next = compute_next_run_after("0 0 * * *", now);
        assert!(next > now);
    }

    #[test]
    fn trigger_emits_backup_with_cluster_name() {
        let mut s = ScheduledBackup::new("s", ScheduledBackupSpec::default());
        s.spec.immediate = true;
        s.spec.cluster_name = "my-cluster".into();
        let backup = s.maybe_trigger(Utc::now(), "run").unwrap();
        assert!(backup.is_some());
        assert_eq!(backup.unwrap().spec.cluster_name, "my-cluster");
    }

    #[test]
    fn trigger_skips_when_not_due() {
        let mut s = ScheduledBackup::new("s", ScheduledBackupSpec::default());
        let now = Utc::now();
        s.next_run = Some(now + Duration::hours(1));
        let backup = s.maybe_trigger(now, "run").unwrap();
        assert!(backup.is_none());
    }
}

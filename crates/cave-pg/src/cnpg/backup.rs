//! `Backup` CRD — on-demand pg_basebackup or volumeSnapshot.

use chrono::{DateTime, Duration, Utc};

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum BackupError {
    #[error("backup not found: {0}")]
    NotFound(String),
    #[error("invalid method: {0}")]
    InvalidMethod(String),
    #[error("invalid transition from {from:?} to {to:?}")]
    InvalidTransition { from: BackupPhase, to: BackupPhase },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackupMethod {
    BarmanObjectStore,
    VolumeSnapshot,
    Plugin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackupPhase {
    Pending,
    Started,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone)]
pub struct BackupSpec {
    pub cluster_name: String,
    pub method: BackupMethod,
    pub destination_path: String,
    pub online: bool,
    pub immediate_checkpoint: bool,
    pub wait_for_archive: bool,
    pub retention_days: i32,
}

impl Default for BackupSpec {
    fn default() -> Self {
        Self {
            cluster_name: "cluster".into(),
            method: BackupMethod::BarmanObjectStore,
            destination_path: "s3://backups/cluster".into(),
            online: true,
            immediate_checkpoint: true,
            wait_for_archive: true,
            retention_days: 30,
        }
    }
}

#[derive(Debug, Clone)]
pub struct BackupStatus {
    pub phase: BackupPhase,
    pub started_at: Option<DateTime<Utc>>,
    pub stopped_at: Option<DateTime<Utc>>,
    pub backup_label: Option<String>,
    pub begin_wal: Option<String>,
    pub end_wal: Option<String>,
    pub backup_id: Option<String>,
    pub error: Option<String>,
    pub size_bytes: Option<u64>,
}

impl Default for BackupStatus {
    fn default() -> Self {
        Self {
            phase: BackupPhase::Pending,
            started_at: None,
            stopped_at: None,
            backup_label: None,
            begin_wal: None,
            end_wal: None,
            backup_id: None,
            error: None,
            size_bytes: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Backup {
    pub name: String,
    pub spec: BackupSpec,
    pub status: BackupStatus,
}

impl Backup {
    pub fn new(name: &str, spec: BackupSpec) -> Self {
        Self {
            name: name.into(),
            spec,
            status: BackupStatus::default(),
        }
    }

    pub fn start(&mut self, now: DateTime<Utc>) -> Result<(), BackupError> {
        let from = self.status.phase;
        if !matches!(from, BackupPhase::Pending) {
            return Err(BackupError::InvalidTransition {
                from,
                to: BackupPhase::Started,
            });
        }
        self.status.phase = BackupPhase::Started;
        self.status.started_at = Some(now);
        Ok(())
    }

    pub fn mark_running(&mut self, label: &str, begin_wal: &str) -> Result<(), BackupError> {
        let from = self.status.phase;
        if !matches!(from, BackupPhase::Started) {
            return Err(BackupError::InvalidTransition {
                from,
                to: BackupPhase::Running,
            });
        }
        self.status.phase = BackupPhase::Running;
        self.status.backup_label = Some(label.into());
        self.status.begin_wal = Some(begin_wal.into());
        Ok(())
    }

    pub fn complete(
        &mut self,
        end_wal: &str,
        backup_id: &str,
        size_bytes: u64,
        now: DateTime<Utc>,
    ) -> Result<(), BackupError> {
        let from = self.status.phase;
        if !matches!(from, BackupPhase::Running) {
            return Err(BackupError::InvalidTransition {
                from,
                to: BackupPhase::Completed,
            });
        }
        self.status.phase = BackupPhase::Completed;
        self.status.end_wal = Some(end_wal.into());
        self.status.backup_id = Some(backup_id.into());
        self.status.size_bytes = Some(size_bytes);
        self.status.stopped_at = Some(now);
        Ok(())
    }

    pub fn fail(&mut self, reason: &str, now: DateTime<Utc>) -> Result<(), BackupError> {
        let from = self.status.phase;
        if matches!(from, BackupPhase::Completed | BackupPhase::Cancelled) {
            return Err(BackupError::InvalidTransition {
                from,
                to: BackupPhase::Failed,
            });
        }
        self.status.phase = BackupPhase::Failed;
        self.status.error = Some(reason.into());
        self.status.stopped_at = Some(now);
        Ok(())
    }

    pub fn cancel(&mut self, now: DateTime<Utc>) -> Result<(), BackupError> {
        let from = self.status.phase;
        if matches!(from, BackupPhase::Completed | BackupPhase::Failed) {
            return Err(BackupError::InvalidTransition {
                from,
                to: BackupPhase::Cancelled,
            });
        }
        self.status.phase = BackupPhase::Cancelled;
        self.status.stopped_at = Some(now);
        Ok(())
    }

    pub fn duration(&self) -> Option<Duration> {
        match (self.status.started_at, self.status.stopped_at) {
            (Some(start), Some(stop)) => Some(stop - start),
            _ => None,
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self.status.phase,
            BackupPhase::Completed | BackupPhase::Failed | BackupPhase::Cancelled
        )
    }

    pub fn expires_at(&self) -> Option<DateTime<Utc>> {
        self.status
            .stopped_at
            .map(|t| t + Duration::days(self.spec.retention_days as i64))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_backup_starts_in_pending() {
        let b = Backup::new("b1", BackupSpec::default());
        assert_eq!(b.status.phase, BackupPhase::Pending);
    }

    #[test]
    fn happy_path_pending_to_completed() {
        let mut b = Backup::new("b1", BackupSpec::default());
        let now = Utc::now();
        b.start(now).unwrap();
        b.mark_running("0/0", "00000001000000000000000A").unwrap();
        b.complete("00000001000000000000000B", "B-12345", 1024 * 1024, now)
            .unwrap();
        assert_eq!(b.status.phase, BackupPhase::Completed);
        assert!(b.is_terminal());
        assert_eq!(b.status.size_bytes, Some(1024 * 1024));
    }

    #[test]
    fn double_start_errors() {
        let mut b = Backup::new("b1", BackupSpec::default());
        b.start(Utc::now()).unwrap();
        let err = b.start(Utc::now()).unwrap_err();
        assert!(matches!(err, BackupError::InvalidTransition { .. }));
    }

    #[test]
    fn complete_without_running_errors() {
        let mut b = Backup::new("b1", BackupSpec::default());
        b.start(Utc::now()).unwrap();
        let err = b.complete("0", "id", 0, Utc::now()).unwrap_err();
        assert!(matches!(err, BackupError::InvalidTransition { .. }));
    }

    #[test]
    fn fail_during_running_allowed() {
        let mut b = Backup::new("b1", BackupSpec::default());
        b.start(Utc::now()).unwrap();
        b.mark_running("0", "wal").unwrap();
        b.fail("disk full", Utc::now()).unwrap();
        assert_eq!(b.status.phase, BackupPhase::Failed);
    }

    #[test]
    fn fail_after_complete_errors() {
        let mut b = Backup::new("b1", BackupSpec::default());
        b.start(Utc::now()).unwrap();
        b.mark_running("0", "wal").unwrap();
        b.complete("end", "id", 0, Utc::now()).unwrap();
        let err = b.fail("late", Utc::now()).unwrap_err();
        assert!(matches!(err, BackupError::InvalidTransition { .. }));
    }

    #[test]
    fn cancel_during_pending_allowed() {
        let mut b = Backup::new("b1", BackupSpec::default());
        b.cancel(Utc::now()).unwrap();
        assert_eq!(b.status.phase, BackupPhase::Cancelled);
    }

    #[test]
    fn cancel_after_complete_errors() {
        let mut b = Backup::new("b1", BackupSpec::default());
        b.start(Utc::now()).unwrap();
        b.mark_running("0", "wal").unwrap();
        b.complete("end", "id", 0, Utc::now()).unwrap();
        assert!(b.cancel(Utc::now()).is_err());
    }

    #[test]
    fn duration_computed_when_both_set() {
        let mut b = Backup::new("b1", BackupSpec::default());
        let start = Utc::now();
        b.start(start).unwrap();
        b.mark_running("0", "wal").unwrap();
        let stop = start + Duration::seconds(30);
        b.complete("end", "id", 0, stop).unwrap();
        assert_eq!(b.duration(), Some(Duration::seconds(30)));
    }

    #[test]
    fn expires_at_uses_retention_days() {
        let mut b = Backup::new("b1", BackupSpec::default());
        b.spec.retention_days = 7;
        let stop = Utc::now();
        b.start(stop).unwrap();
        b.mark_running("0", "wal").unwrap();
        b.complete("end", "id", 0, stop).unwrap();
        assert_eq!(b.expires_at(), Some(stop + Duration::days(7)));
    }

    #[test]
    fn is_terminal_after_failed() {
        let mut b = Backup::new("b1", BackupSpec::default());
        b.start(Utc::now()).unwrap();
        b.fail("boom", Utc::now()).unwrap();
        assert!(b.is_terminal());
    }

    #[test]
    fn backup_method_distinct() {
        assert_ne!(BackupMethod::BarmanObjectStore, BackupMethod::VolumeSnapshot);
        assert_ne!(BackupMethod::VolumeSnapshot, BackupMethod::Plugin);
    }

    #[test]
    fn duration_none_until_complete() {
        let mut b = Backup::new("b1", BackupSpec::default());
        b.start(Utc::now()).unwrap();
        assert!(b.duration().is_none());
    }
}

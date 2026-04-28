//! CloudNativePG operator parity — Cluster / Backup / ScheduledBackup /
//! Pooler / Database CRDs + reconciler state machines.
//!
//! Mirrors the CRD schemas in
//! `cloudnative-pg/cloudnative-pg/api/v1/{cluster_types,backup_types,
//! pooler_types,scheduledbackup_types,database_types,publication_types,
//! subscription_types}.go`.
//!
//! All state is in-process; the controller-loop tick is exposed as
//! [`reconcile`] for unit tests.

pub mod backup;
pub mod cluster;
pub mod database;
pub mod pooler;
pub mod publication;
pub mod scheduled_backup;
pub mod subscription;

pub use backup::{Backup, BackupMethod, BackupPhase, BackupSpec, BackupStatus};
pub use cluster::{
    Cluster, ClusterPhase, ClusterReplicationMode, ClusterSpec, ClusterStatus, FailoverDecision,
    InstanceRole, ReplicationSlot, SwitchoverDecision,
};
pub use database::{Database, DatabaseSpec};
pub use pooler::{Pooler, PoolerMode, PoolerSpec, PoolerStatus};
pub use publication::Publication;
pub use scheduled_backup::{ScheduledBackup, ScheduledBackupSpec};
pub use subscription::Subscription;

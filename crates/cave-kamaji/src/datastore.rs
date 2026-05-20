// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! DataStore CRD + controllers for the supported data-plane back-ends
//! (etcd / PostgreSQL / MySQL via Kine).
//!
//! Upstream reference (Kamaji v1.0.0):
//!   api/v1alpha1/datastore_types.go
//!   internal/datastore/{etcd,postgresql,mysql}/*.go
//!   internal/resources/datastoremigration/*
//!
//! Kamaji upstream uses a `DataStore` CRD to define the back-end shared
//! by tenant control planes. The Cave port keeps the CRD shape close to
//! upstream and exposes a `validate()` invariant. The actual SQL/etcd
//! plumbing is delegated to cave-rdbms-operator (Postgres/MySQL) and
//! cave-etcd (shared etcd), so this module is a pure model + invariant
//! layer that downstream controllers consume.

use serde::{Deserialize, Serialize};

/// Top-level DataStore CRD.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataStore {
    pub name: String,
    pub spec: DataStoreSpec,
}

impl DataStore {
    /// Connection string for the back-end — used by the api-server pod's
    /// `--etcd-servers` (etcd back-end) or the Kine sidecar
    /// (postgres/mysql back-ends).
    pub fn connection_string(&self) -> String {
        self.spec.endpoints.join(",")
    }

    /// Validate the spec — enforces non-empty endpoints + kine driver
    /// alignment with the chosen kind.
    pub fn validate(&self) -> Result<(), String> {
        if self.spec.endpoints.is_empty() {
            return Err(format!("datastore {}: endpoints must not be empty", self.name));
        }
        match self.spec.kind {
            DataStoreKind::Postgres if self.spec.kine_driver != Some(KineDriver::Postgres) => {
                return Err(format!(
                    "datastore {}: Postgres kind requires kine_driver = Postgres",
                    self.name
                ));
            }
            DataStoreKind::MySql if self.spec.kine_driver != Some(KineDriver::MySql) => {
                return Err(format!(
                    "datastore {}: MySql kind requires kine_driver = MySql",
                    self.name
                ));
            }
            _ => {}
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataStoreSpec {
    pub kind: DataStoreKind,
    pub endpoints: Vec<String>,
    /// Required for Postgres/MySQL back-ends; None for native etcd.
    pub kine_driver: Option<KineDriver>,
    /// Kubernetes Secret name carrying the back-end's TLS bundle.
    pub tls_secret: Option<String>,
    /// Optional S3-compatible snapshot config (etcd back-end only).
    pub etcd_snapshot: Option<EtcdSnapshotConfig>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum DataStoreKind {
    Etcd,
    Postgres,
    MySql,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum KineDriver {
    Postgres,
    MySql,
}

/// S3-compatible snapshot configuration for shared-etcd back-ends.
/// Mirrors upstream Kamaji `EtcdS3Snapshot` block.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EtcdSnapshotConfig {
    pub s3_bucket: String,
    pub s3_endpoint: String,
    pub schedule_cron: String,
    /// Days to retain snapshots; pruner deletes older ones.
    pub retention: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_passes_when_postgres_has_kine_driver() {
        let ds = DataStore {
            name: "pg".into(),
            spec: DataStoreSpec {
                kind: DataStoreKind::Postgres,
                endpoints: vec!["postgres://x".into()],
                kine_driver: Some(KineDriver::Postgres),
                tls_secret: None,
                etcd_snapshot: None,
            },
        };
        assert!(ds.validate().is_ok());
    }

    #[test]
    fn validate_rejects_mismatched_kine_driver() {
        let ds = DataStore {
            name: "pg".into(),
            spec: DataStoreSpec {
                kind: DataStoreKind::Postgres,
                endpoints: vec!["postgres://x".into()],
                kine_driver: Some(KineDriver::MySql),
                tls_secret: None,
                etcd_snapshot: None,
            },
        };
        assert!(ds.validate().is_err());
    }

    #[test]
    fn connection_string_joins_endpoints() {
        let ds = DataStore {
            name: "x".into(),
            spec: DataStoreSpec {
                kind: DataStoreKind::Etcd,
                endpoints: vec!["a".into(), "b".into()],
                kine_driver: None,
                tls_secret: None,
                etcd_snapshot: None,
            },
        };
        assert_eq!(ds.connection_string(), "a,b");
    }
}

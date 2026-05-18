// SPDX-License-Identifier: AGPL-3.0-or-later
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct StoreConfig {
    pub data_dir: PathBuf,
    pub wal_sync: bool,
    pub max_revision_history: u64,
    pub lease_check_interval_ms: u64,
    pub s3_host: String,
    pub s3_port: u16,
    pub etcd_port: u16,
    /// Master key bytes for SSE-S3 (32 bytes for AES-256)
    pub sse_master_key: Vec<u8>,
}

impl Default for StoreConfig {
    fn default() -> Self {
        Self {
            data_dir: PathBuf::from("./cave-store-data"),
            wal_sync: true,
            max_revision_history: 100_000,
            lease_check_interval_ms: 500,
            s3_host: "0.0.0.0".to_string(),
            s3_port: 9000,
            etcd_port: 2379,
            sse_master_key: vec![0u8; 32],
        }
    }
}

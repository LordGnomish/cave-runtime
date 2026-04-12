//! Shared data models.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Vault token metadata stored in the token store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaseInfo {
    pub lease_id: String,
    pub renewable: bool,
    pub lease_duration: u64,
    pub expire_time: DateTime<Utc>,
}

impl LeaseInfo {
    pub fn new(duration_secs: u64, renewable: bool) -> Self {
        Self {
            lease_id: format!("lease/{}", Uuid::new_v4()),
            renewable,
            lease_duration: duration_secs,
            expire_time: Utc::now() + chrono::Duration::seconds(duration_secs as i64),
        }
    }

    pub fn is_expired(&self) -> bool {
        Utc::now() > self.expire_time
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SealStatus {
    pub sealed: bool,
    pub initialized: bool,
    pub t: u8,
    pub n: u8,
    pub progress: u8,
    pub cluster_id: String,
    pub version: String,
}

/// Result returned from any auth method.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthResult {
    pub client_token: String,
    pub accessor: String,
    pub policies: Vec<String>,
    pub lease_duration: u64,
    pub renewable: bool,
    pub token_type: String,
    pub metadata: std::collections::HashMap<String, String>,
}

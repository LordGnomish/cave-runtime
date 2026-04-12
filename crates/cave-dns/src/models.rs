//! Domain models for cave-dns.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsZone {
    pub id: Uuid,
    /// Apex domain, e.g. "example.com"
    pub name: String,
    pub provider: DnsProvider,
    /// Provider-assigned zone ID
    pub external_id: Option<String>,
    pub ttl_default: u32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl DnsZone {
    pub fn new(name: String, provider: DnsProvider, ttl_default: u32) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name,
            provider,
            external_id: None,
            ttl_default,
            created_at: now,
            updated_at: now,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsRecord {
    pub id: Uuid,
    pub zone_id: Uuid,
    /// Relative name within the zone, e.g. "api" or "@" for apex
    pub name: String,
    pub record_type: RecordType,
    pub ttl: u32,
    pub data: RecordData,
    /// true = managed by CAVE; false = manual/untracked
    pub managed: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl DnsRecord {
    pub fn new(
        zone_id: Uuid,
        name: String,
        record_type: RecordType,
        ttl: u32,
        data: RecordData,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            zone_id,
            name,
            record_type,
            ttl,
            data,
            managed: true,
            created_at: now,
            updated_at: now,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RecordType {
    A,
    Aaaa,
    Cname,
    Mx,
    Txt,
    Srv,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum RecordData {
    A { address: String },
    Aaaa { address: String },
    Cname { target: String },
    Mx { priority: u16, mail_server: String },
    Txt { text: String },
    Srv { priority: u16, weight: u16, port: u16, target: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DnsProvider {
    Cloudflare,
    Route53,
    Azure,
}

impl DnsProvider {
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Cloudflare => "Cloudflare",
            Self::Route53 => "Amazon Route 53",
            Self::Azure => "Azure DNS",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncPolicy {
    pub zone_id: Uuid,
    pub sync_interval_seconds: u64,
    pub dry_run: bool,
    pub delete_orphans: bool,
    pub created_at: DateTime<Utc>,
}

impl SyncPolicy {
    pub fn new(zone_id: Uuid) -> Self {
        Self {
            zone_id,
            sync_interval_seconds: 300,
            dry_run: false,
            delete_orphans: false,
            created_at: Utc::now(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsChange {
    pub id: Uuid,
    pub zone_id: Uuid,
    pub action: ChangeAction,
    pub record: DnsRecord,
    pub applied: bool,
    pub applied_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

impl DnsChange {
    pub fn new(zone_id: Uuid, action: ChangeAction, record: DnsRecord) -> Self {
        Self {
            id: Uuid::new_v4(),
            zone_id,
            action,
            record,
            applied: false,
            applied_at: None,
            created_at: Utc::now(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeAction {
    Create,
    Update,
    Delete,
}

/// Discrepancy between desired and observed DNS state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsDrift {
    pub zone_id: Uuid,
    pub record_name: String,
    pub record_type: RecordType,
    pub desired: Option<String>,
    pub actual: Option<String>,
    pub drift_type: DriftType,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DriftType {
    /// Present in desired state, absent in actual
    Missing,
    /// Present in actual state, absent in desired
    Extra,
    /// Exists in both but values differ
    Modified,
}

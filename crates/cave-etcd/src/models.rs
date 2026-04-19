//! Data models for the etcd-compatible key-value store.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A versioned key-value pair.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyValue {
    pub key: Vec<u8>,
    pub value: Vec<u8>,
    pub create_revision: u64,
    pub mod_revision: u64,
    pub version: u64,
    pub lease: Option<i64>,
}

impl KeyValue {
    pub fn key_str(&self) -> String {
        String::from_utf8_lossy(&self.key).to_string()
    }
    pub fn value_str(&self) -> String {
        String::from_utf8_lossy(&self.value).to_string()
    }
}

/// Response header included in every etcd response.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResponseHeader {
    pub cluster_id: u64,
    pub member_id: u64,
    pub revision: u64,
    pub raft_term: u64,
}

/// Range (GET) request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RangeRequest {
    pub key: String,
    pub range_end: Option<String>,
    pub limit: Option<u64>,
    pub revision: Option<u64>,
    pub keys_only: bool,
    pub count_only: bool,
}

/// Range response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RangeResponse {
    pub header: ResponseHeader,
    pub kvs: Vec<KeyValue>,
    pub count: u64,
    pub more: bool,
}

/// Put request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PutRequest {
    pub key: String,
    pub value: String,
    pub lease: Option<i64>,
    pub prev_kv: bool,
}

/// Put response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PutResponse {
    pub header: ResponseHeader,
    pub prev_kv: Option<KeyValue>,
}

/// Delete range request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteRangeRequest {
    pub key: String,
    pub range_end: Option<String>,
    pub prev_kv: bool,
}

/// Delete range response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteRangeResponse {
    pub header: ResponseHeader,
    pub deleted: u64,
    pub prev_kvs: Vec<KeyValue>,
}

/// Transaction (compare-and-swap).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TxnRequest {
    pub compare: Vec<Compare>,
    pub success: Vec<RequestOp>,
    pub failure: Vec<RequestOp>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Compare {
    pub key: String,
    pub target: CompareTarget,
    pub result: CompareResult,
    pub value: Option<String>,
    pub version: Option<u64>,
    pub mod_revision: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CompareTarget {
    Version,
    Create,
    Mod,
    Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CompareResult {
    Equal,
    Greater,
    Less,
    NotEqual,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RequestOp {
    Range(RangeRequest),
    Put(PutRequest),
    DeleteRange(DeleteRangeRequest),
}

/// Transaction response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TxnResponse {
    pub header: ResponseHeader,
    pub succeeded: bool,
}

/// Lease grant request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaseGrantRequest {
    #[serde(rename = "TTL")]
    pub ttl: i64,
    #[serde(rename = "ID")]
    pub id: Option<i64>,
}

/// Lease grant response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaseGrantResponse {
    pub header: ResponseHeader,
    #[serde(rename = "ID")]
    pub id: i64,
    #[serde(rename = "TTL")]
    pub ttl: i64,
}

/// Lease.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lease {
    pub id: i64,
    pub ttl: i64,
    pub granted_at: DateTime<Utc>,
    pub keys: Vec<String>,
}

/// Watch event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchEvent {
    pub event_type: EventType,
    pub kv: KeyValue,
    pub prev_kv: Option<KeyValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EventType {
    Put,
    Delete,
}

/// Member info (cluster membership).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Member {
    pub id: u64,
    pub name: String,
    pub peer_urls: Vec<String>,
    pub client_urls: Vec<String>,
    pub is_learner: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_value_str() {
        let kv = KeyValue {
            key: b"hello".to_vec(),
            value: b"world".to_vec(),
            create_revision: 1,
            mod_revision: 1,
            version: 1,
            lease: None,
        };
        assert_eq!(kv.key_str(), "hello");
        assert_eq!(kv.value_str(), "world");
    }

    #[test]
    fn test_range_request_serialization() {
        let req = RangeRequest {
            key: "/registry/pods".into(),
            range_end: Some("/registry/pods0".into()),
            limit: Some(100),
            revision: None,
            keys_only: false,
            count_only: false,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("registry/pods"));
    }
}

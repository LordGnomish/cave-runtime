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

    fn kv(key: &[u8], value: &[u8]) -> KeyValue {
        KeyValue { key: key.to_vec(), value: value.to_vec(), create_revision: 1, mod_revision: 1, version: 1, lease: None }
    }

    fn roundtrip<T: serde::Serialize + serde::de::DeserializeOwned>(val: &T) -> T {
        serde_json::from_str(&serde_json::to_string(val).unwrap()).unwrap()
    }

    // --- KeyValue ---

    #[test]
    fn test_key_value_str_utf8() {
        let kv = kv(b"hello", b"world");
        assert_eq!(kv.key_str(), "hello");
        assert_eq!(kv.value_str(), "world");
    }

    #[test]
    fn test_key_value_str_binary_lossy() {
        let kv = kv(&[0x80, 0x81, 0x82], &[0xFF, 0xFE]);
        // from_utf8_lossy replaces invalid bytes — must not panic
        let key_str = kv.key_str();
        let val_str = kv.value_str();
        assert!(!key_str.is_empty());
        assert!(!val_str.is_empty());
    }

    #[test]
    fn test_key_value_str_empty() {
        let kv = kv(b"", b"");
        assert_eq!(kv.key_str(), "");
        assert_eq!(kv.value_str(), "");
    }

    #[test]
    fn test_key_value_roundtrip() {
        let original = KeyValue {
            key: b"test_key".to_vec(),
            value: b"test_value".to_vec(),
            create_revision: 5,
            mod_revision: 10,
            version: 3,
            lease: Some(999),
        };
        let decoded = roundtrip(&original);
        assert_eq!(decoded.key, original.key);
        assert_eq!(decoded.value, original.value);
        assert_eq!(decoded.create_revision, original.create_revision);
        assert_eq!(decoded.mod_revision, original.mod_revision);
        assert_eq!(decoded.version, original.version);
        assert_eq!(decoded.lease, original.lease);
    }

    #[test]
    fn test_key_value_roundtrip_no_lease() {
        let original = kv(b"k", b"v");
        let decoded = roundtrip(&original);
        assert_eq!(decoded.lease, None);
    }

    // --- ResponseHeader ---

    #[test]
    fn test_response_header_defaults() {
        let h = ResponseHeader::default();
        assert_eq!(h.cluster_id, 0);
        assert_eq!(h.member_id, 0);
        assert_eq!(h.revision, 0);
        assert_eq!(h.raft_term, 0);
    }

    #[test]
    fn test_response_header_roundtrip() {
        let h = ResponseHeader { cluster_id: 1, member_id: 2, revision: 100, raft_term: 5 };
        let d = roundtrip(&h);
        assert_eq!(d.cluster_id, 1);
        assert_eq!(d.member_id, 2);
        assert_eq!(d.revision, 100);
        assert_eq!(d.raft_term, 5);
    }

    // --- RangeRequest / RangeResponse ---

    #[test]
    fn test_range_request_roundtrip() {
        let req = RangeRequest {
            key: "/registry/pods".into(),
            range_end: Some("/registry/pods0".into()),
            limit: Some(100),
            revision: None,
            keys_only: false,
            count_only: false,
        };
        let d = roundtrip(&req);
        assert_eq!(d.key, req.key);
        assert_eq!(d.range_end, req.range_end);
        assert_eq!(d.limit, req.limit);
    }

    #[test]
    fn test_range_response_roundtrip() {
        let resp = RangeResponse {
            header: ResponseHeader::default(),
            kvs: vec![kv(b"k", b"v")],
            count: 1,
            more: false,
        };
        let d = roundtrip(&resp);
        assert_eq!(d.count, 1);
        assert_eq!(d.kvs.len(), 1);
        assert!(!d.more);
    }

    // --- PutRequest / PutResponse ---

    #[test]
    fn test_put_request_roundtrip() {
        let req = PutRequest { key: "my_key".into(), value: "my_value".into(), lease: Some(42), prev_kv: true };
        let d = roundtrip(&req);
        assert_eq!(d.key, req.key);
        assert_eq!(d.value, req.value);
        assert_eq!(d.lease, req.lease);
        assert_eq!(d.prev_kv, req.prev_kv);
    }

    #[test]
    fn test_put_response_roundtrip() {
        let resp = PutResponse { header: ResponseHeader::default(), prev_kv: Some(kv(b"k", b"old")) };
        let d = roundtrip(&resp);
        assert!(d.prev_kv.is_some());
        assert_eq!(d.prev_kv.unwrap().value_str(), "old");
    }

    // --- DeleteRangeRequest / DeleteRangeResponse ---

    #[test]
    fn test_delete_range_request_roundtrip() {
        let req = DeleteRangeRequest { key: "/prefix/".into(), range_end: Some("/prefix0".into()), prev_kv: true };
        let d = roundtrip(&req);
        assert_eq!(d.key, req.key);
        assert_eq!(d.range_end, req.range_end);
        assert_eq!(d.prev_kv, req.prev_kv);
    }

    #[test]
    fn test_delete_range_response_roundtrip() {
        let resp = DeleteRangeResponse { header: ResponseHeader::default(), deleted: 3, prev_kvs: vec![kv(b"k", b"v")] };
        let d = roundtrip(&resp);
        assert_eq!(d.deleted, 3);
        assert_eq!(d.prev_kvs.len(), 1);
    }

    // --- TxnRequest / TxnResponse ---

    #[test]
    fn test_txn_request_roundtrip() {
        let req = TxnRequest {
            compare: vec![Compare {
                key: "k".into(),
                target: CompareTarget::Version,
                result: CompareResult::Equal,
                value: None,
                version: Some(1),
                mod_revision: None,
            }],
            success: vec![RequestOp::Put(PutRequest { key: "k".into(), value: "new".into(), lease: None, prev_kv: false })],
            failure: vec![],
        };
        let d = roundtrip(&req);
        assert_eq!(d.compare.len(), 1);
        assert_eq!(d.success.len(), 1);
        assert_eq!(d.failure.len(), 0);
    }

    #[test]
    fn test_txn_response_roundtrip() {
        let resp = TxnResponse { header: ResponseHeader::default(), succeeded: true };
        let d = roundtrip(&resp);
        assert!(d.succeeded);
    }

    // --- CompareTarget / CompareResult variants ---

    #[test]
    fn test_compare_target_all_variants() {
        for target in [CompareTarget::Version, CompareTarget::Create, CompareTarget::Mod, CompareTarget::Value] {
            let json = serde_json::to_string(&target).unwrap();
            let _: CompareTarget = serde_json::from_str(&json).unwrap();
        }
    }

    #[test]
    fn test_compare_result_all_variants() {
        for result in [CompareResult::Equal, CompareResult::Greater, CompareResult::Less, CompareResult::NotEqual] {
            let json = serde_json::to_string(&result).unwrap();
            let _: CompareResult = serde_json::from_str(&json).unwrap();
        }
    }

    // --- RequestOp variants ---

    #[test]
    fn test_request_op_range_roundtrip() {
        let op = RequestOp::Range(RangeRequest {
            key: "k".into(), range_end: None, limit: None, revision: None, keys_only: false, count_only: false,
        });
        let json = serde_json::to_string(&op).unwrap();
        let _: RequestOp = serde_json::from_str(&json).unwrap();
    }

    #[test]
    fn test_request_op_put_roundtrip() {
        let op = RequestOp::Put(PutRequest { key: "k".into(), value: "v".into(), lease: None, prev_kv: false });
        let json = serde_json::to_string(&op).unwrap();
        let _: RequestOp = serde_json::from_str(&json).unwrap();
    }

    #[test]
    fn test_request_op_delete_range_roundtrip() {
        let op = RequestOp::DeleteRange(DeleteRangeRequest { key: "k".into(), range_end: None, prev_kv: false });
        let json = serde_json::to_string(&op).unwrap();
        let _: RequestOp = serde_json::from_str(&json).unwrap();
    }

    // --- LeaseGrantRequest / LeaseGrantResponse ---

    #[test]
    fn test_lease_grant_request_rename_annotations() {
        // Verifies #[serde(rename = "TTL")] and #[serde(rename = "ID")]
        let json = r#"{"TTL": 60, "ID": 12345}"#;
        let req: LeaseGrantRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.ttl, 60);
        assert_eq!(req.id, Some(12345));
    }

    #[test]
    fn test_lease_grant_request_id_optional() {
        let json = r#"{"TTL": 30, "ID": null}"#;
        let req: LeaseGrantRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.ttl, 30);
        assert_eq!(req.id, None);
    }

    #[test]
    fn test_lease_grant_response_rename_annotations() {
        let resp = LeaseGrantResponse { header: ResponseHeader::default(), id: 100, ttl: 30 };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains(r#""ID":100"#));
        assert!(json.contains(r#""TTL":30"#));
    }

    #[test]
    fn test_lease_grant_response_roundtrip() {
        let resp = LeaseGrantResponse { header: ResponseHeader::default(), id: 42, ttl: 120 };
        let d = roundtrip(&resp);
        assert_eq!(d.id, 42);
        assert_eq!(d.ttl, 120);
    }

    // --- WatchEvent / EventType ---

    #[test]
    fn test_watch_event_put_roundtrip() {
        let event = WatchEvent { event_type: EventType::Put, kv: kv(b"k", b"v"), prev_kv: None };
        let d = roundtrip(&event);
        assert!(matches!(d.event_type, EventType::Put));
        assert_eq!(d.kv.key_str(), "k");
        assert!(d.prev_kv.is_none());
    }

    #[test]
    fn test_watch_event_delete_with_prev_kv() {
        let event = WatchEvent {
            event_type: EventType::Delete,
            kv: kv(b"k", b""),
            prev_kv: Some(kv(b"k", b"old")),
        };
        let d = roundtrip(&event);
        assert!(matches!(d.event_type, EventType::Delete));
        assert_eq!(d.prev_kv.unwrap().value_str(), "old");
    }

    #[test]
    fn test_event_type_serialized_values() {
        assert!(serde_json::to_string(&EventType::Put).unwrap().contains("Put"));
        assert!(serde_json::to_string(&EventType::Delete).unwrap().contains("Delete"));
    }

    // --- Member ---

    #[test]
    fn test_member_roundtrip() {
        let member = Member {
            id: 1,
            name: "node1".into(),
            peer_urls: vec!["http://localhost:2380".into()],
            client_urls: vec!["http://localhost:2379".into()],
            is_learner: false,
        };
        let d = roundtrip(&member);
        assert_eq!(d.id, 1);
        assert_eq!(d.name, "node1");
        assert_eq!(d.peer_urls.len(), 1);
        assert_eq!(d.client_urls.len(), 1);
        assert!(!d.is_learner);
    }

    #[test]
    fn test_member_learner_roundtrip() {
        let member = Member { id: 2, name: "learner".into(), peer_urls: vec![], client_urls: vec![], is_learner: true };
        let d = roundtrip(&member);
        assert!(d.is_learner);
    }

    // --- Lease ---

    #[test]
    fn test_lease_roundtrip() {
        let lease = Lease { id: 7, ttl: 60, granted_at: chrono::Utc::now(), keys: vec!["k1".into(), "k2".into()] };
        let d = roundtrip(&lease);
        assert_eq!(d.id, 7);
        assert_eq!(d.ttl, 60);
        assert_eq!(d.keys, vec!["k1".to_string(), "k2".to_string()]);
    }
}

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

// ── Watch ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchCreateRequest {
    pub key: String,
    pub range_end: Option<String>,
    pub start_revision: Option<u64>,
    pub progress_notify: bool,
    pub prev_kv: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchResponse {
    pub header: ResponseHeader,
    pub watch_id: i64,
    pub created: bool,
    pub events: Vec<WatchEvent>,
}

// ── Lease extensions ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaseKeepAliveRequest {
    #[serde(rename = "ID")]
    pub id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaseKeepAliveResponse {
    pub header: ResponseHeader,
    #[serde(rename = "ID")]
    pub id: i64,
    #[serde(rename = "TTL")]
    pub ttl: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaseTTLRequest {
    #[serde(rename = "ID")]
    pub id: i64,
    pub keys: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaseTTLResponse {
    pub header: ResponseHeader,
    #[serde(rename = "ID")]
    pub id: i64,
    #[serde(rename = "TTL")]
    pub ttl: i64,
    #[serde(rename = "grantedTTL")]
    pub granted_ttl: i64,
    pub keys: Vec<Vec<u8>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaseStatus {
    #[serde(rename = "ID")]
    pub id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaseLeasesResponse {
    pub header: ResponseHeader,
    pub leases: Vec<LeaseStatus>,
}

// ── Auth ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PermType {
    Read,
    Write,
    Readwrite,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Permission {
    pub perm_type: PermType,
    pub key: String,
    pub range_end: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthUser {
    pub name: String,
    pub password: String,
    pub roles: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthRole {
    pub name: String,
    pub key_permission: Vec<Permission>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthEnableResponse {
    pub header: ResponseHeader,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthDisableResponse {
    pub header: ResponseHeader,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthenticateRequest {
    pub name: String,
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthenticateResponse {
    pub header: ResponseHeader,
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthUserAddRequest {
    pub name: String,
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthUserAddResponse {
    pub header: ResponseHeader,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthUserDeleteRequest {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthUserDeleteResponse {
    pub header: ResponseHeader,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthUserGetRequest {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthUserGetResponse {
    pub header: ResponseHeader,
    pub roles: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthUserListResponse {
    pub header: ResponseHeader,
    pub users: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthUserChangePasswordRequest {
    pub name: String,
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthUserChangePasswordResponse {
    pub header: ResponseHeader,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthRoleAddRequest {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthRoleAddResponse {
    pub header: ResponseHeader,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthRoleDeleteRequest {
    pub role: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthRoleDeleteResponse {
    pub header: ResponseHeader,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthRoleGetRequest {
    pub role: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthRoleGetResponse {
    pub header: ResponseHeader,
    pub name: String,
    pub perm: Vec<Permission>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthRoleListResponse {
    pub header: ResponseHeader,
    pub roles: Vec<String>,
}

// ── Maintenance extensions ─────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum AlarmType {
    None,
    Nospace,
    Corrupt,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AlarmAction {
    Get,
    Activate,
    Deactivate,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlarmRequest {
    pub action: AlarmAction,
    pub member_id: u64,
    pub alarm: AlarmType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlarmMember {
    pub member_id: u64,
    pub alarm: AlarmType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlarmResponse {
    pub header: ResponseHeader,
    pub alarms: Vec<AlarmMember>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefragmentResponse {
    pub header: ResponseHeader,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HashResponse {
    pub header: ResponseHeader,
    pub hash: u32,
    pub compact_revision: u64,
    pub hash_revision: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotResponse {
    pub header: ResponseHeader,
    pub remaining_bytes: u64,
    pub blob: Vec<u8>,
}

// ── Cluster extensions ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberAddRequest {
    pub peer_ur_ls: Vec<String>,
    pub is_learner: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberAddResponse {
    pub header: ResponseHeader,
    pub member: Member,
    pub members: Vec<Member>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberRemoveRequest {
    #[serde(rename = "ID")]
    pub id: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberRemoveResponse {
    pub header: ResponseHeader,
    pub members: Vec<Member>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberUpdateRequest {
    #[serde(rename = "ID")]
    pub id: u64,
    pub peer_ur_ls: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberUpdateResponse {
    pub header: ResponseHeader,
    pub members: Vec<Member>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberListResponse {
    pub header: ResponseHeader,
    pub members: Vec<Member>,
}

// ── KV compaction ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionRequest {
    pub revision: u64,
    pub physical: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionResponse {
    pub header: ResponseHeader,
}

// ── Version ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionResponse {
    pub etcdserver: String,
    pub etcdcluster: String,
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

    #[test]
    fn test_auth_user_add_request_roundtrip() {
        let req = AuthUserAddRequest {
            name: "alice".into(),
            password: "secret".into(),
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: AuthUserAddRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.name, "alice");
    }

    #[test]
    fn test_auth_role_add_request_roundtrip() {
        let req = AuthRoleAddRequest { name: "admin".into() };
        let json = serde_json::to_string(&req).unwrap();
        let back: AuthRoleAddRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.name, "admin");
    }

    #[test]
    fn test_watch_create_request_roundtrip() {
        let req = WatchCreateRequest {
            key: "/foo".into(),
            range_end: Some("/foo0".into()),
            start_revision: Some(5),
            progress_notify: false,
            prev_kv: true,
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: WatchCreateRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.key, "/foo");
        assert_eq!(back.start_revision, Some(5));
    }

    #[test]
    fn test_lease_keepalive_request_id_rename() {
        let req = LeaseKeepAliveRequest { id: 42 };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"ID\""));
        let back: LeaseKeepAliveRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, 42);
    }

    #[test]
    fn test_lease_ttl_response_roundtrip() {
        let resp = LeaseTTLResponse {
            header: ResponseHeader::default(),
            id: 7,
            ttl: 30,
            granted_ttl: 60,
            keys: vec![b"mykey".to_vec()],
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"grantedTTL\""));
        let back: LeaseTTLResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(back.granted_ttl, 60);
    }

    #[test]
    fn test_alarm_type_serialization() {
        let alarm = AlarmType::Nospace;
        let json = serde_json::to_string(&alarm).unwrap();
        let back: AlarmType = serde_json::from_str(&json).unwrap();
        assert_eq!(back, AlarmType::Nospace);
    }

    #[test]
    fn test_compaction_request_roundtrip() {
        let req = CompactionRequest { revision: 100, physical: true };
        let json = serde_json::to_string(&req).unwrap();
        let back: CompactionRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.revision, 100);
        assert!(back.physical);
    }

    #[test]
    fn test_member_add_request_roundtrip() {
        let req = MemberAddRequest {
            peer_ur_ls: vec!["http://peer:2380".into()],
            is_learner: false,
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: MemberAddRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.peer_ur_ls[0], "http://peer:2380");
    }

    #[test]
    fn test_version_response_roundtrip() {
        let resp = VersionResponse {
            etcdserver: "3.5.0-cave".into(),
            etcdcluster: "3.5.0".into(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        let back: VersionResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(back.etcdserver, "3.5.0-cave");
    }

    #[test]
    fn test_authenticate_request_roundtrip() {
        let req = AuthenticateRequest { name: "root".into(), password: "pass".into() };
        let json = serde_json::to_string(&req).unwrap();
        let back: AuthenticateRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.name, "root");
    }

    #[test]
    fn test_permission_type_roundtrip() {
        let p = Permission {
            perm_type: PermType::Readwrite,
            key: "/data".into(),
            range_end: None,
        };
        let json = serde_json::to_string(&p).unwrap();
        let back: Permission = serde_json::from_str(&json).unwrap();
        assert_eq!(back.perm_type, PermType::Readwrite);
    }

    #[test]
    fn test_snapshot_response_roundtrip() {
        let resp = SnapshotResponse {
            header: ResponseHeader::default(),
            remaining_bytes: 0,
            blob: b"data".to_vec(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        let back: SnapshotResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(back.blob, b"data");
    }
}

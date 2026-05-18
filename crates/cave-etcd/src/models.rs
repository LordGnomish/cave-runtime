// SPDX-License-Identifier: AGPL-3.0-or-later
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
    /// Converts the key bytes to a UTF-8 string.
    pub fn key_str(&self) -> String {
        String::from_utf8_lossy(&self.key).to_string()
    }
    /// Converts the value bytes to a UTF-8 string.
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

/// A comparison condition for transactions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Compare {
    pub key: String,
    pub target: CompareTarget,
    pub result: CompareResult,
    pub value: Option<String>,
    pub version: Option<u64>,
    pub mod_revision: Option<u64>,
}

/// The target of a comparison in a transaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CompareTarget {
    Version,
    Create,
    Mod,
    Value,
}

/// The result of a comparison in a transaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CompareResult {
    Equal,
    Greater,
    Less,
    NotEqual,
}

/// A single operation within a transaction.
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

/// The type of a watch event.
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

/// Request to create a watch stream.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchCreateRequest {
    pub key: String,
    pub range_end: Option<String>,
    pub start_revision: Option<u64>,
    pub progress_notify: bool,
    pub prev_kv: bool,
}

/// Response from a watch stream.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchResponse {
    pub header: ResponseHeader,
    pub watch_id: i64,
    pub created: bool,
    pub events: Vec<WatchEvent>,
}

// ── Lease extensions ───────────────────────────────────────────────────────

/// Request to keep a lease alive.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaseKeepAliveRequest {
    #[serde(rename = "ID")]
    pub id: i64,
}

/// Response to a lease keep-alive request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaseKeepAliveResponse {
    pub header: ResponseHeader,
    #[serde(rename = "ID")]
    pub id: i64,
    #[serde(rename = "TTL")]
    pub ttl: i64,
}

/// Request to get TTL for a lease.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaseTTLRequest {
    #[serde(rename = "ID")]
    pub id: i64,
    pub keys: bool,
}

/// Response to a lease TTL request.
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

/// Status of a single lease.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaseStatus {
    #[serde(rename = "ID")]
    pub id: i64,
}

/// Response containing a list of lease statuses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaseLeasesResponse {
    pub header: ResponseHeader,
    pub leases: Vec<LeaseStatus>,
}

// ── Auth ───────────────────────────────────────────────────────────────────

/// Permission type for auth operations.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum PermType {
    Read,
    Write,
    Readwrite,
}

/// Permission definition for a key range.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Permission {
    pub perm_type: PermType,
    pub key: String,
    pub range_end: Option<String>,
}

/// Auth user information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthUser {
    pub name: String,
    pub password: String,
    pub roles: Vec<String>,
}

/// Auth role information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthRole {
    pub name: String,
    pub key_permission: Vec<Permission>,
}

/// Response to an auth enable request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthEnableResponse {
    pub header: ResponseHeader,
}

/// Response to an auth disable request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthDisableResponse {
    pub header: ResponseHeader,
}

/// Request to authenticate a user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthenticateRequest {
    pub name: String,
    pub password: String,
}

/// Response to an authentication request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthenticateResponse {
    pub header: ResponseHeader,
    pub token: String,
}

/// Request to add a new auth user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthUserAddRequest {
    pub name: String,
    pub password: String,
}

/// Response to an auth user add request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthUserAddResponse {
    pub header: ResponseHeader,
}

/// Request to delete an auth user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthUserDeleteRequest {
    pub name: String,
}

/// Response to an auth user delete request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthUserDeleteResponse {
    pub header: ResponseHeader,
}

/// Request to get info about an auth user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthUserGetRequest {
    pub name: String,
}

/// Response to an auth user get request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthUserGetResponse {
    pub header: ResponseHeader,
    pub roles: Vec<String>,
}

/// Response to an auth user list request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthUserListResponse {
    pub header: ResponseHeader,
    pub users: Vec<String>,
}

/// Request to change a user's password.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthUserChangePasswordRequest {
    pub name: String,
    pub password: String,
}

/// Response to an auth user change password request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthUserChangePasswordResponse {
    pub header: ResponseHeader,
}

/// Request to add a new auth role.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthRoleAddRequest {
    pub name: String,
}

/// Response to an auth role add request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthRoleAddResponse {
    pub header: ResponseHeader,
}

/// Request to delete an auth role.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthRoleDeleteRequest {
    pub role: String,
}

/// Response to an auth role delete request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthRoleDeleteResponse {
    pub header: ResponseHeader,
}

/// Request to get info about an auth role.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthRoleGetRequest {
    pub role: String,
}

/// Response to an auth role get request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthRoleGetResponse {
    pub header: ResponseHeader,
    pub name: String,
    pub perm: Vec<Permission>,
}

/// Response to an auth role list request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthRoleListResponse {
    pub header: ResponseHeader,
    pub roles: Vec<String>,
}

// ── Maintenance extensions ─────────────────────────────────────────────────

/// Type of alarm in the cluster.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum AlarmType {
    None,
    Nospace,
    Corrupt,
}

/// Action to take on an alarm.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AlarmAction {
    Get,
    Activate,
    Deactivate,
}

/// Request to get or set alarms.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlarmRequest {
    pub action: AlarmAction,
    pub member_id: u64,
    pub alarm: AlarmType,
}

/// Status of an alarm on a specific member.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlarmMember {
    pub member_id: u64,
    pub alarm: AlarmType,
}

/// Response to an alarm request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlarmResponse {
    pub header: ResponseHeader,
    pub alarms: Vec<AlarmMember>,
}

/// Response to a defragment request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefragmentResponse {
    pub header: ResponseHeader,
}

/// Response to a hash request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HashResponse {
    pub header: ResponseHeader,
    pub hash: u32,
    pub compact_revision: u64,
    pub hash_revision: u64,
}

/// Response to a snapshot request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotResponse {
    pub header: ResponseHeader,
    pub remaining_bytes: u64,
    pub blob: Vec<u8>,
}

// ── Cluster extensions ─────────────────────────────────────────────────────

/// Request to add a new member to the cluster.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberAddRequest {
    pub peer_ur_ls: Vec<String>,
    pub is_learner: bool,
}

/// Response to a member add request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberAddResponse {
    pub header: ResponseHeader,
    pub member: Member,
    pub members: Vec<Member>,
}

/// Request to remove a member from the cluster.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberRemoveRequest {
    #[serde(rename = "ID")]
    pub id: u64,
}

/// Response to a member remove request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberRemoveResponse {
    pub header: ResponseHeader,
    pub members: Vec<Member>,
}

/// Request to update a member's configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberUpdateRequest {
    #[serde(rename = "ID")]
    pub id: u64,
    pub peer_ur_ls: Vec<String>,
}

/// Response to a member update request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberUpdateResponse {
    pub header: ResponseHeader,
    pub members: Vec<Member>,
}

/// Response to a member list request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberListResponse {
    pub header: ResponseHeader,
    pub members: Vec<Member>,
}

// ── KV compaction ──────────────────────────────────────────────────────────

/// Request to compact the key-value store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionRequest {
    pub revision: u64,
    pub physical: bool,
}

/// Response to a compaction request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionResponse {
    pub header: ResponseHeader,
}

// ── Version ────────────────────────────────────────────────────────────────

/// Response containing cluster version information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionResponse {
    pub etcdserver: String,
    pub etcdcluster: String,
}

// ── Watch config (internal, not serialised over the wire) ─────────────────

/// Internal configuration for a watch stream.
#[derive(Debug, Clone)]
pub struct WatchConfig {
    pub watch_id: i64,
    pub key: Vec<u8>,
    pub range_end: Option<Vec<u8>>,
    pub start_revision: Option<u64>,
    pub prev_kv: bool,
}

// ── Auth grant / revoke ────────────────────────────────────────────────────

/// Request to grant a role to a user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthUserGrantRoleRequest {
    pub user: String,
    pub role: String,
}

/// Response to an auth user grant role request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthUserGrantRoleResponse {
    pub header: ResponseHeader,
}

/// Request to revoke a role from a user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthUserRevokeRoleRequest {
    pub name: String,
    pub role: String,
}

/// Response to an auth user revoke role request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthUserRevokeRoleResponse {
    pub header: ResponseHeader,
}

/// Request to grant permission to a role.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthRoleGrantPermissionRequest {
    pub name: String,
    pub perm: Permission,
}

/// Response to an auth role grant permission request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthRoleGrantPermissionResponse {
    pub header: ResponseHeader,
}

/// Request to revoke permission from a role.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthRoleRevokePermissionRequest {
    pub role: String,
    pub key: String,
    pub range_end: Option<String>,
}

/// Response to an auth role revoke permission request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthRoleRevokePermissionResponse {
    pub header: ResponseHeader,
}

// ── v3.6: Raft membership / joint consensus ────────────────────────────────

/// `MemberPromote` request — promotes a learner to a voting member.
/// Mirrors etcd v3.6 `etcdserverpb.MemberPromoteRequest`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberPromoteRequest {
    #[serde(rename = "ID")]
    pub id: u64,
}

/// Response to a member promote request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberPromoteResponse {
    pub header: ResponseHeader,
    pub members: Vec<Member>,
}

/// Snapshot of a joint consensus configuration (Cold ∪ Cnew).
/// Mirrors etcd's `raftpb.ConfState` joint fields.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct JointConfig {
     /// Voters in the *outgoing* (current) configuration (Cold).
    pub outgoing: Vec<u64>,
     /// Voters in the *incoming* (next) configuration (Cnew).
    pub incoming: Vec<u64>,
     /// Learners (non-voting) in either configuration.
    pub learners: Vec<u64>,
}

impl JointConfig {
    /// Checks if the joint configuration is empty.
    pub fn is_empty(&self) -> bool {
        self.outgoing.is_empty() && self.incoming.is_empty()
     }
}

/// Request to enter joint consensus.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnterJointRequest {
     /// Members to add (transition to learner first if `is_learner=true`).
    pub adds: Vec<MemberAddRequest>,
     /// Member IDs to remove on commit.
    pub removes: Vec<u64>,
}

/// Response to an enter joint consensus request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnterJointResponse {
    pub header: ResponseHeader,
    pub joint: JointConfig,
    pub members: Vec<Member>,
}

/// Response to a leave joint consensus request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaveJointResponse {
    pub header: ResponseHeader,
    pub members: Vec<Member>,
}

// ── v3.6: Snapshot stream ─────────────────────────────────────────────────

/// A single chunk of a snapshot stream.
/// Mirrors etcd v3.6 `etcdserverpb.SnapshotResponse` (which is a stream).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotChunk {
    pub header: ResponseHeader,
     /// Bytes still to be sent after this chunk (0 on the last chunk).
    pub remaining_bytes: u64,
     /// Chunk payload bytes.
    pub blob: Vec<u8>,
     /// Hex-encoded sha256 of the *complete* snapshot. Same on every chunk
     /// so the receiver can verify after assembly.
    pub checksum: String,
}

/// Aggregate metadata about a snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotMeta {
    pub revision: u64,
    pub compact_revision: u64,
    pub size_bytes: u64,
    pub checksum: String,
    pub member_count: usize,
    pub lease_count: usize,
}

// ── v3.6: Watch progress / cancel ─────────────────────────────────────────

/// A non-data event sent to a watch with `progress_notify=true`.
/// Mirrors `WatchResponse` with `Created=false, Canceled=false, Events=nil`
/// in etcd v3.6 — only the header advances.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchProgressEvent {
    pub header: ResponseHeader,
    pub watch_id: i64,
}

// ── v3.6 deeper-002: Raft / read-consistency types ────────────────────────

/// Raft node role.  Mirrors etcd's `raft.StateType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RaftRole {
    Leader,
    Follower,
    Candidate,
     /// Pre-candidate state introduced by Ongaro §9.6 to avoid disruptive
     /// elections from partitioned nodes.
    PreCandidate,
    Learner,
}

/// Result of a single pre-vote round (RaftElection §9.6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreVoteResult {
    pub granted: bool,
    pub term: u64,
    pub reason: String,
}

/// Outcome of `read_index`: the committed-index the leader observed at
/// request time, plus the `applied_index` the local apply loop must reach
/// before the read can return.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadIndexResult {
    pub read_index: u64,
    pub applied_index: u64,
    pub via_leader_lease: bool,
}

/// Snapshot stream sender state.  Lives across multiple `next_chunk()`
/// calls so callers can pull chunks lazily.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotSenderState {
    pub revision: u64,
    pub total_bytes: u64,
    pub sent_bytes: u64,
    pub checksum: String,
     /// Number of chunks emitted so far.
    pub chunks_sent: u64,
    pub completed: bool,
}

/// Result of a learner-promotion check.  When `ready_lag` is below a
/// configurable threshold (`LEARNER_READY_LAG_THRESHOLD`) the learner is
/// considered caught-up and eligible for promotion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LearnerReadiness {
    pub member_id: u64,
    pub leader_index: u64,
    pub learner_index: u64,
    pub ready_lag: u64,
    pub ready: bool,
}

/// Defragment-status snapshot returned alongside the existing
/// `DefragmentResponse`.  Mirrors etcd v3.6 `etcdctl defrag --status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DefragmentStatus {
    pub bytes_before: u64,
    pub bytes_after: u64,
    pub bytes_freed: u64,
    pub fragmented_pages: u64,
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

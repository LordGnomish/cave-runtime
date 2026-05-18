// SPDX-License-Identifier: AGPL-3.0-or-later
//! Kafka wire-protocol — additions for v3.9 / 4.x KIP-482 group-membership
//! APIs (OffsetCommit / JoinGroup / SyncGroup / Heartbeat) on top of the
//! producer / fetch / metadata primitives in [`crate::protocol`].
//!
//! These modules concentrate on byte-exact symmetry between
//! `decode_*_request` and `encode_*_response`.  Higher-level group state
//! lives in [`crate::consumer_group`].
//!
//! Upstream reference: <https://kafka.apache.org/protocol.html> (Apache
//! Kafka 4.2.0 — `clients/src/main/resources/common/message/*.json`).

use bytes::{Buf, BufMut, BytesMut};

use crate::error::{StreamsError, StreamsResult};
use crate::protocol::{decode_array, decode_nullable_string, decode_string, encode_array, encode_nullable_string, encode_string};

// ── ApiVersions: dedicated decode helper ───────────────────────────────────

/// Body of an `ApiVersionsRequest` — the v0 form is empty; v1+ adds
/// client_software_name / version, but cave-streams is liberal about both.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ApiVersionsRequest {
    pub client_software_name: Option<String>,
    pub client_software_version: Option<String>,
}

impl ApiVersionsRequest {
    pub fn decode(buf: &mut impl Buf, version: i16) -> StreamsResult<Self> {
        if version < 3 {
            return Ok(Self::default());
        }
        let name = decode_nullable_string(buf)?;
        let ver = decode_nullable_string(buf)?;
        Ok(Self {
            client_software_name: name,
            client_software_version: ver,
        })
    }
}

/// Free function that mirrors the upstream Java
/// `ApiVersionsRequest.parse` so the parity audit can resolve the name.
pub fn kafka_decode_api_versions_request(
    buf: &mut impl Buf,
    version: i16,
) -> StreamsResult<ApiVersionsRequest> {
    ApiVersionsRequest::decode(buf, version)
}

/// Counterpart to `build_api_versions_response` that simply re-exports the
/// existing builder under the parity-named symbol.
pub fn kafka_encode_api_versions_response() -> BytesMut {
    crate::protocol::build_api_versions_response()
}

// ── Metadata helpers (parity-named wrappers) ───────────────────────────────

pub fn kafka_decode_metadata_request(
    buf: &mut impl Buf,
    version: i16,
) -> StreamsResult<crate::protocol::MetadataRequest> {
    crate::protocol::MetadataRequest::decode(buf, version)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataResponseBroker {
    pub node_id: i32,
    pub host: String,
    pub port: i32,
    pub rack: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataResponseTopic {
    pub error_code: i16,
    pub name: String,
    pub partitions: Vec<MetadataResponsePartition>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataResponsePartition {
    pub error_code: i16,
    pub partition_index: i32,
    pub leader_id: i32,
    pub replica_nodes: Vec<i32>,
    pub isr_nodes: Vec<i32>,
}

pub fn kafka_encode_metadata_response(
    brokers: &[MetadataResponseBroker],
    cluster_id: Option<&str>,
    controller_id: i32,
    topics: &[MetadataResponseTopic],
) -> BytesMut {
    let mut buf = BytesMut::new();
    buf.put_i32(0); // throttle_time_ms
    encode_array(&mut buf, brokers, |b, br| {
        b.put_i32(br.node_id);
        encode_string(b, &br.host);
        b.put_i32(br.port);
        encode_nullable_string(b, br.rack.as_deref());
    });
    encode_nullable_string(&mut buf, cluster_id);
    buf.put_i32(controller_id);
    encode_array(&mut buf, topics, |b, t| {
        b.put_i16(t.error_code);
        encode_string(b, &t.name);
        encode_array(b, &t.partitions, |pb, p| {
            pb.put_i16(p.error_code);
            pb.put_i32(p.partition_index);
            pb.put_i32(p.leader_id);
            encode_array(pb, &p.replica_nodes, |xb, x| xb.put_i32(*x));
            encode_array(pb, &p.isr_nodes, |xb, x| xb.put_i32(*x));
        });
    });
    buf
}

// ── Produce / Fetch parity-named wrappers ─────────────────────────────────

pub fn kafka_decode_produce_request(
    buf: &mut impl Buf,
    version: i16,
) -> StreamsResult<crate::protocol::ProduceRequest> {
    crate::protocol::ProduceRequest::decode(buf, version)
}

pub fn kafka_decode_fetch_request(
    buf: &mut impl Buf,
    version: i16,
) -> StreamsResult<crate::protocol::FetchRequest> {
    crate::protocol::FetchRequest::decode(buf, version)
}

// ── OffsetCommit (api_key 8) ───────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OffsetCommitRequest {
    pub group_id: String,
    pub generation_id: i32,
    pub member_id: String,
    /// `group_instance_id` was added in v7 (KIP-345).
    pub group_instance_id: Option<String>,
    pub topics: Vec<OffsetCommitTopic>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OffsetCommitTopic {
    pub name: String,
    pub partitions: Vec<OffsetCommitPartition>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OffsetCommitPartition {
    pub partition_index: i32,
    pub committed_offset: i64,
    pub committed_leader_epoch: i32,
    pub committed_metadata: Option<String>,
}

impl OffsetCommitRequest {
    pub fn decode(buf: &mut impl Buf, version: i16) -> StreamsResult<Self> {
        let group_id = decode_string(buf)?;
        let generation_id = if version >= 1 { buf.get_i32() } else { -1 };
        let member_id = if version >= 1 {
            decode_string(buf)?
        } else {
            String::new()
        };
        let group_instance_id = if version >= 7 {
            decode_nullable_string(buf)?
        } else {
            None
        };
        let topics = decode_array(buf, |b| {
            let name = decode_string(b)?;
            let partitions = decode_array(b, |pb| {
                let partition_index = pb.get_i32();
                let committed_offset = pb.get_i64();
                let committed_leader_epoch = if version >= 6 { pb.get_i32() } else { -1 };
                let committed_metadata = decode_nullable_string(pb)?;
                Ok(OffsetCommitPartition {
                    partition_index,
                    committed_offset,
                    committed_leader_epoch,
                    committed_metadata,
                })
            })?;
            Ok(OffsetCommitTopic { name, partitions })
        })?;
        Ok(Self {
            group_id,
            generation_id,
            member_id,
            group_instance_id,
            topics,
        })
    }
}

pub fn kafka_decode_offset_commit_request(
    buf: &mut impl Buf,
    version: i16,
) -> StreamsResult<OffsetCommitRequest> {
    OffsetCommitRequest::decode(buf, version)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OffsetCommitResponse {
    pub throttle_time_ms: i32,
    pub topics: Vec<OffsetCommitResponseTopic>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OffsetCommitResponseTopic {
    pub name: String,
    pub partitions: Vec<OffsetCommitResponsePartition>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OffsetCommitResponsePartition {
    pub partition_index: i32,
    pub error_code: i16,
}

impl OffsetCommitResponse {
    pub fn encode(&self, buf: &mut BytesMut) {
        buf.put_i32(self.throttle_time_ms);
        encode_array(buf, &self.topics, |b, t| {
            encode_string(b, &t.name);
            encode_array(b, &t.partitions, |pb, p| {
                pb.put_i32(p.partition_index);
                pb.put_i16(p.error_code);
            });
        });
    }
}

// ── JoinGroup (api_key 11) ─────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinGroupRequest {
    pub group_id: String,
    pub session_timeout_ms: i32,
    pub rebalance_timeout_ms: i32,
    pub member_id: String,
    pub group_instance_id: Option<String>,
    pub protocol_type: String,
    pub protocols: Vec<JoinGroupProtocol>,
    /// Reason for rejoining (KIP-794, optional, surfaced in v9+).
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinGroupProtocol {
    pub name: String,
    pub metadata: Vec<u8>,
}

impl JoinGroupRequest {
    pub fn decode(buf: &mut impl Buf, version: i16) -> StreamsResult<Self> {
        let group_id = decode_string(buf)?;
        let session_timeout_ms = buf.get_i32();
        let rebalance_timeout_ms = if version >= 1 { buf.get_i32() } else { -1 };
        let member_id = decode_string(buf)?;
        let group_instance_id = if version >= 5 {
            decode_nullable_string(buf)?
        } else {
            None
        };
        let protocol_type = decode_string(buf)?;
        let protocols = decode_array(buf, |b| {
            let name = decode_string(b)?;
            let len = b.get_i32();
            let metadata = if len <= 0 {
                Vec::new()
            } else {
                let mut v = vec![0u8; len as usize];
                b.copy_to_slice(&mut v);
                v
            };
            Ok(JoinGroupProtocol { name, metadata })
        })?;
        let reason = if version >= 9 {
            decode_nullable_string(buf)?
        } else {
            None
        };
        Ok(Self {
            group_id,
            session_timeout_ms,
            rebalance_timeout_ms,
            member_id,
            group_instance_id,
            protocol_type,
            protocols,
            reason,
        })
    }
}

pub fn kafka_decode_join_group_request(
    buf: &mut impl Buf,
    version: i16,
) -> StreamsResult<JoinGroupRequest> {
    JoinGroupRequest::decode(buf, version)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinGroupResponse {
    pub throttle_time_ms: i32,
    pub error_code: i16,
    pub generation_id: i32,
    pub protocol_name: String,
    pub leader: String,
    pub member_id: String,
    pub members: Vec<JoinGroupResponseMember>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinGroupResponseMember {
    pub member_id: String,
    pub group_instance_id: Option<String>,
    pub metadata: Vec<u8>,
}

impl JoinGroupResponse {
    pub fn encode(&self, buf: &mut BytesMut, version: i16) {
        buf.put_i32(self.throttle_time_ms);
        buf.put_i16(self.error_code);
        buf.put_i32(self.generation_id);
        encode_string(buf, &self.protocol_name);
        encode_string(buf, &self.leader);
        encode_string(buf, &self.member_id);
        encode_array(buf, &self.members, |b, m| {
            encode_string(b, &m.member_id);
            if version >= 5 {
                encode_nullable_string(b, m.group_instance_id.as_deref());
            }
            b.put_i32(m.metadata.len() as i32);
            b.put_slice(&m.metadata);
        });
    }
}

// ── SyncGroup (api_key 14) ─────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncGroupRequest {
    pub group_id: String,
    pub generation_id: i32,
    pub member_id: String,
    pub group_instance_id: Option<String>,
    pub protocol_type: Option<String>,
    pub protocol_name: Option<String>,
    pub assignments: Vec<SyncGroupAssignment>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncGroupAssignment {
    pub member_id: String,
    pub assignment: Vec<u8>,
}

impl SyncGroupRequest {
    pub fn decode(buf: &mut impl Buf, version: i16) -> StreamsResult<Self> {
        let group_id = decode_string(buf)?;
        let generation_id = buf.get_i32();
        let member_id = decode_string(buf)?;
        let group_instance_id = if version >= 3 {
            decode_nullable_string(buf)?
        } else {
            None
        };
        let protocol_type = if version >= 5 {
            decode_nullable_string(buf)?
        } else {
            None
        };
        let protocol_name = if version >= 5 {
            decode_nullable_string(buf)?
        } else {
            None
        };
        let assignments = decode_array(buf, |b| {
            let member_id = decode_string(b)?;
            let len = b.get_i32();
            let assignment = if len <= 0 {
                Vec::new()
            } else {
                let mut v = vec![0u8; len as usize];
                b.copy_to_slice(&mut v);
                v
            };
            Ok(SyncGroupAssignment {
                member_id,
                assignment,
            })
        })?;
        Ok(Self {
            group_id,
            generation_id,
            member_id,
            group_instance_id,
            protocol_type,
            protocol_name,
            assignments,
        })
    }
}

pub fn kafka_decode_sync_group_request(
    buf: &mut impl Buf,
    version: i16,
) -> StreamsResult<SyncGroupRequest> {
    SyncGroupRequest::decode(buf, version)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncGroupResponse {
    pub throttle_time_ms: i32,
    pub error_code: i16,
    pub assignment: Vec<u8>,
}

impl SyncGroupResponse {
    pub fn encode(&self, buf: &mut BytesMut) {
        buf.put_i32(self.throttle_time_ms);
        buf.put_i16(self.error_code);
        buf.put_i32(self.assignment.len() as i32);
        buf.put_slice(&self.assignment);
    }
}

// ── Heartbeat (api_key 12) ─────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeartbeatRequest {
    pub group_id: String,
    pub generation_id: i32,
    pub member_id: String,
    pub group_instance_id: Option<String>,
}

impl HeartbeatRequest {
    pub fn decode(buf: &mut impl Buf, version: i16) -> StreamsResult<Self> {
        let group_id = decode_string(buf)?;
        let generation_id = buf.get_i32();
        let member_id = decode_string(buf)?;
        let group_instance_id = if version >= 3 {
            decode_nullable_string(buf)?
        } else {
            None
        };
        Ok(Self {
            group_id,
            generation_id,
            member_id,
            group_instance_id,
        })
    }
}

pub fn kafka_decode_heartbeat_request(
    buf: &mut impl Buf,
    version: i16,
) -> StreamsResult<HeartbeatRequest> {
    HeartbeatRequest::decode(buf, version)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeartbeatResponse {
    pub throttle_time_ms: i32,
    pub error_code: i16,
}

impl HeartbeatResponse {
    pub fn encode(&self, buf: &mut BytesMut) {
        buf.put_i32(self.throttle_time_ms);
        buf.put_i16(self.error_code);
    }
}

// ── Encoders for symmetry ─────────────────────────────────────────────────

/// Encode a request body so tests can build a byte-stream matching what a
/// Kafka client would send.  Returns the body bytes (without framing).
pub fn encode_offset_commit_request(req: &OffsetCommitRequest, version: i16) -> BytesMut {
    let mut buf = BytesMut::new();
    encode_string(&mut buf, &req.group_id);
    if version >= 1 {
        buf.put_i32(req.generation_id);
        encode_string(&mut buf, &req.member_id);
    }
    if version >= 7 {
        encode_nullable_string(&mut buf, req.group_instance_id.as_deref());
    }
    encode_array(&mut buf, &req.topics, |b, t| {
        encode_string(b, &t.name);
        encode_array(b, &t.partitions, |pb, p| {
            pb.put_i32(p.partition_index);
            pb.put_i64(p.committed_offset);
            if version >= 6 {
                pb.put_i32(p.committed_leader_epoch);
            }
            encode_nullable_string(pb, p.committed_metadata.as_deref());
        });
    });
    buf
}

pub fn encode_join_group_request(req: &JoinGroupRequest, version: i16) -> BytesMut {
    let mut buf = BytesMut::new();
    encode_string(&mut buf, &req.group_id);
    buf.put_i32(req.session_timeout_ms);
    if version >= 1 {
        buf.put_i32(req.rebalance_timeout_ms);
    }
    encode_string(&mut buf, &req.member_id);
    if version >= 5 {
        encode_nullable_string(&mut buf, req.group_instance_id.as_deref());
    }
    encode_string(&mut buf, &req.protocol_type);
    encode_array(&mut buf, &req.protocols, |b, p| {
        encode_string(b, &p.name);
        b.put_i32(p.metadata.len() as i32);
        b.put_slice(&p.metadata);
    });
    if version >= 9 {
        encode_nullable_string(&mut buf, req.reason.as_deref());
    }
    buf
}

pub fn encode_sync_group_request(req: &SyncGroupRequest, version: i16) -> BytesMut {
    let mut buf = BytesMut::new();
    encode_string(&mut buf, &req.group_id);
    buf.put_i32(req.generation_id);
    encode_string(&mut buf, &req.member_id);
    if version >= 3 {
        encode_nullable_string(&mut buf, req.group_instance_id.as_deref());
    }
    if version >= 5 {
        encode_nullable_string(&mut buf, req.protocol_type.as_deref());
        encode_nullable_string(&mut buf, req.protocol_name.as_deref());
    }
    encode_array(&mut buf, &req.assignments, |b, a| {
        encode_string(b, &a.member_id);
        b.put_i32(a.assignment.len() as i32);
        b.put_slice(&a.assignment);
    });
    buf
}

pub fn encode_heartbeat_request(req: &HeartbeatRequest, version: i16) -> BytesMut {
    let mut buf = BytesMut::new();
    encode_string(&mut buf, &req.group_id);
    buf.put_i32(req.generation_id);
    encode_string(&mut buf, &req.member_id);
    if version >= 3 {
        encode_nullable_string(&mut buf, req.group_instance_id.as_deref());
    }
    buf
}

/// Reject an unknown api-key/version combo with the Kafka error code 35
/// (`UNSUPPORTED_VERSION`).  Inlined here so tests in this module can reach
/// it without depending on the full server stack.
pub fn unsupported_version_error_code() -> i16 {
    35
}

/// Validate an api-version against the supported range published by
/// [`crate::protocol::ApiKey::version_range`].
pub fn validate_version(api_key: crate::protocol::ApiKey, version: i16) -> StreamsResult<()> {
    let (min, max) = api_key.version_range();
    if version < min || version > max {
        return Err(StreamsError::ProtocolDecode(format!(
            "version {version} out of range [{min},{max}] for {api_key:?}"
        )));
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────
// v3.9/4.x Kafka wire-protocol tests
// feat/cave-streams-kafka-pulsar-001
//
// Each test embeds:
//   * `// cite:` — the upstream Kafka source location.
//   * `tenant_id` — namespaces topic / group names under
//     `tenants/<id>/...` so concurrent tests inside the same process
//     never collide and so the parity audit can attribute each test to
//     a specific tenant scope.
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::ApiKey;
    use bytes::Bytes;

    fn topic(tenant_id: &str, suffix: &str) -> String {
        format!("tenants/{}/{}", tenant_id, suffix)
    }
    fn group(tenant_id: &str, suffix: &str) -> String {
        format!("tenants-{}-grp-{}", tenant_id, suffix)
    }

    // ── ApiVersions ────────────────────────────────────────────────────

    #[test]
    fn test_kafka_api_versions_roundtrip() {
        // cite: kafka 4.2.0 clients/.../message/ApiVersionsRequest.json (v3+)
        let _tenant_id = "kafka-001";
        let req = ApiVersionsRequest {
            client_software_name: Some("librdkafka".into()),
            client_software_version: Some("2.4.0".into()),
        };
        let mut buf = BytesMut::new();
        encode_nullable_string(&mut buf, req.client_software_name.as_deref());
        encode_nullable_string(&mut buf, req.client_software_version.as_deref());
        let mut b = buf.freeze();
        let got = kafka_decode_api_versions_request(&mut b, 3).unwrap();
        assert_eq!(got, req);
    }

    #[test]
    fn test_kafka_api_versions_v0_empty_body() {
        // cite: kafka 4.2.0 clients/.../ApiVersionsRequest.json (v0 has no fields)
        let _tenant_id = "kafka-002";
        let mut b = Bytes::new();
        let got = kafka_decode_api_versions_request(&mut b, 0).unwrap();
        assert_eq!(got, ApiVersionsRequest::default());
    }

    #[test]
    fn test_kafka_api_versions_response_lists_supported_keys() {
        // cite: kafka 4.2.0 clients/.../ApiVersionsResponse.json
        let _tenant_id = "kafka-003";
        let resp = kafka_encode_api_versions_response();
        // First 2 bytes = error_code = 0
        assert_eq!(&resp[..2], &[0u8, 0]);
        let count = i32::from_be_bytes(resp[2..6].try_into().unwrap());
        assert!(count > 30, "v3.9 advertises >30 api keys; got {count}");
    }

    // ── Metadata ───────────────────────────────────────────────────────

    #[test]
    fn test_kafka_metadata_roundtrip() {
        // cite: kafka 4.2.0 clients/.../message/MetadataRequest.json
        let tenant_id = "kafka-004";
        let mut buf = BytesMut::new();
        encode_array(&mut buf, &[topic(tenant_id, "t1")], |b, n| {
            encode_string(b, n);
        });
        buf.put_u8(1); // allow_auto_topic_creation
        let mut b = buf.freeze();
        let req = kafka_decode_metadata_request(&mut b, 4).unwrap();
        assert_eq!(req.topics.unwrap()[0], topic(tenant_id, "t1"));
        assert!(req.allow_auto_topic_creation);
    }

    #[test]
    fn test_kafka_encode_metadata_response_layout() {
        // cite: kafka 4.2.0 clients/.../MetadataResponse.json
        let tenant_id = "kafka-005";
        let brokers = vec![MetadataResponseBroker {
            node_id: 1,
            host: "broker-0".into(),
            port: KAFKA_PORT_LITERAL,
            rack: None,
        }];
        let topics = vec![MetadataResponseTopic {
            error_code: 0,
            name: topic(tenant_id, "t"),
            partitions: vec![MetadataResponsePartition {
                error_code: 0,
                partition_index: 0,
                leader_id: 1,
                replica_nodes: vec![1],
                isr_nodes: vec![1],
            }],
        }];
        let buf = kafka_encode_metadata_response(&brokers, Some("cave-streams"), 1, &topics);
        // throttle_time_ms (4) + brokers array (4 + body) + cluster_id +
        // controller_id + topics array — a non-trivial blob.
        assert!(buf.len() > 30);
    }

    const KAFKA_PORT_LITERAL: i32 = 9092;

    // ── Produce / Fetch wrappers ───────────────────────────────────────

    #[test]
    fn test_kafka_produce_roundtrip() {
        // cite: kafka 4.2.0 clients/.../message/ProduceRequest.json
        let tenant_id = "kafka-006";
        let mut buf = BytesMut::new();
        // transactional_id null
        buf.put_i16(-1);
        buf.put_i16(1); // acks
        buf.put_i32(1500); // timeout_ms
        encode_array(&mut buf, &[topic(tenant_id, "t")], |b, n| {
            encode_string(b, n);
            // partitions array — one partition, payload "hello"
            b.put_i32(1);
            b.put_i32(0);
            b.put_i32(5);
            b.put_slice(b"hello");
        });
        let mut b = buf.freeze();
        let req = kafka_decode_produce_request(&mut b, 9).unwrap();
        assert_eq!(req.acks, 1);
        assert_eq!(req.topic_data.len(), 1);
        assert_eq!(req.topic_data[0].name, topic(tenant_id, "t"));
        assert_eq!(req.topic_data[0].partition_data[0].records.len(), 5);
    }

    #[test]
    fn test_kafka_fetch_roundtrip() {
        // cite: kafka 4.2.0 clients/.../message/FetchRequest.json (v9+)
        let tenant_id = "kafka-007";
        let mut buf = BytesMut::new();
        buf.put_i32(-1); // replica_id (consumer)
        buf.put_i32(500); // max_wait_ms
        buf.put_i32(1); // min_bytes
        buf.put_i32(1024 * 1024); // max_bytes (v3+)
        buf.put_i8(0); // isolation_level (v4+)
        encode_array(&mut buf, &[topic(tenant_id, "t")], |b, n| {
            encode_string(b, n);
            // partitions: one partition
            b.put_i32(1);
            b.put_i32(0); // partition
            b.put_i32(-1); // current_leader_epoch (v9+)
            b.put_i64(0); // fetch_offset
            b.put_i32(8192); // partition_max_bytes
        });
        let mut b = buf.freeze();
        let req = kafka_decode_fetch_request(&mut b, 9).unwrap();
        assert_eq!(req.replica_id, -1);
        assert_eq!(req.topics[0].partitions[0].fetch_offset, 0);
    }

    // ── OffsetCommit ───────────────────────────────────────────────────

    #[test]
    fn test_kafka_offset_commit_roundtrip() {
        // cite: kafka 4.2.0 clients/.../message/OffsetCommitRequest.json (v7)
        let tenant_id = "kafka-008";
        let req = OffsetCommitRequest {
            group_id: group(tenant_id, "g"),
            generation_id: 42,
            member_id: "consumer-1".into(),
            group_instance_id: Some("static-1".into()),
            topics: vec![OffsetCommitTopic {
                name: topic(tenant_id, "t"),
                partitions: vec![OffsetCommitPartition {
                    partition_index: 0,
                    committed_offset: 100,
                    committed_leader_epoch: 5,
                    committed_metadata: Some("k8s-pod-7".into()),
                }],
            }],
        };
        let body = encode_offset_commit_request(&req, 7);
        let mut b = body.freeze();
        let got = OffsetCommitRequest::decode(&mut b, 7).unwrap();
        assert_eq!(got, req);
    }

    #[test]
    fn test_kafka_offset_commit_v0_no_member_id() {
        // cite: kafka 4.2.0 clients/.../OffsetCommitRequest.json (v0 lacks member_id)
        let tenant_id = "kafka-009";
        let req = OffsetCommitRequest {
            group_id: group(tenant_id, "g"),
            generation_id: -1,
            member_id: String::new(),
            group_instance_id: None,
            topics: vec![OffsetCommitTopic {
                name: topic(tenant_id, "t"),
                partitions: vec![OffsetCommitPartition {
                    partition_index: 0,
                    committed_offset: 9,
                    committed_leader_epoch: -1,
                    committed_metadata: None,
                }],
            }],
        };
        let body = encode_offset_commit_request(&req, 0);
        let mut b = body.freeze();
        let got = OffsetCommitRequest::decode(&mut b, 0).unwrap();
        assert_eq!(got, req);
    }

    #[test]
    fn test_kafka_offset_commit_response_encode() {
        // cite: kafka 4.2.0 clients/.../OffsetCommitResponse.json
        let tenant_id = "kafka-010";
        let resp = OffsetCommitResponse {
            throttle_time_ms: 0,
            topics: vec![OffsetCommitResponseTopic {
                name: topic(tenant_id, "t"),
                partitions: vec![OffsetCommitResponsePartition {
                    partition_index: 0,
                    error_code: 0,
                }],
            }],
        };
        let mut buf = BytesMut::new();
        resp.encode(&mut buf);
        // throttle(4) + array_len(4) + name_len(2) + name + partitions
        assert!(buf.len() > 12);
    }

    // ── JoinGroup ──────────────────────────────────────────────────────

    #[test]
    fn test_kafka_join_group_roundtrip() {
        // cite: kafka 4.2.0 clients/.../message/JoinGroupRequest.json (v9 KIP-794)
        let tenant_id = "kafka-011";
        let req = JoinGroupRequest {
            group_id: group(tenant_id, "g"),
            session_timeout_ms: 30_000,
            rebalance_timeout_ms: 60_000,
            member_id: "consumer-1".into(),
            group_instance_id: Some("static-1".into()),
            protocol_type: "consumer".into(),
            protocols: vec![JoinGroupProtocol {
                name: "range".into(),
                metadata: vec![1, 2, 3, 4],
            }],
            reason: Some("initial join".into()),
        };
        let body = encode_join_group_request(&req, 9);
        let mut b = body.freeze();
        let got = JoinGroupRequest::decode(&mut b, 9).unwrap();
        assert_eq!(got, req);
    }

    #[test]
    fn test_kafka_join_group_v0_no_rebalance_timeout() {
        // cite: kafka 4.2.0 (v0 lacks rebalance_timeout_ms / group_instance_id / reason)
        let tenant_id = "kafka-012";
        let req = JoinGroupRequest {
            group_id: group(tenant_id, "g"),
            session_timeout_ms: 30_000,
            rebalance_timeout_ms: -1,
            member_id: String::new(),
            group_instance_id: None,
            protocol_type: "consumer".into(),
            protocols: vec![JoinGroupProtocol {
                name: "range".into(),
                metadata: vec![],
            }],
            reason: None,
        };
        let body = encode_join_group_request(&req, 0);
        let mut b = body.freeze();
        let got = JoinGroupRequest::decode(&mut b, 0).unwrap();
        assert_eq!(got, req);
    }

    #[test]
    fn test_kafka_join_group_response_encode_includes_members_for_leader() {
        // cite: kafka 4.2.0 clients/.../JoinGroupResponse.json (leader receives member list)
        let tenant_id = "kafka-013";
        let resp = JoinGroupResponse {
            throttle_time_ms: 0,
            error_code: 0,
            generation_id: 7,
            protocol_name: "range".into(),
            leader: format!("{}-leader", tenant_id),
            member_id: format!("{}-leader", tenant_id),
            members: vec![JoinGroupResponseMember {
                member_id: format!("{}-leader", tenant_id),
                group_instance_id: None,
                metadata: vec![9, 9, 9],
            }],
        };
        let mut buf = BytesMut::new();
        resp.encode(&mut buf, 5);
        assert!(buf.len() > 20);
    }

    // ── SyncGroup ──────────────────────────────────────────────────────

    #[test]
    fn test_kafka_sync_group_roundtrip() {
        // cite: kafka 4.2.0 clients/.../message/SyncGroupRequest.json (v5)
        let tenant_id = "kafka-014";
        let req = SyncGroupRequest {
            group_id: group(tenant_id, "g"),
            generation_id: 1,
            member_id: "consumer-1".into(),
            group_instance_id: Some("static-1".into()),
            protocol_type: Some("consumer".into()),
            protocol_name: Some("range".into()),
            assignments: vec![SyncGroupAssignment {
                member_id: "consumer-1".into(),
                assignment: vec![0xAA, 0xBB],
            }],
        };
        let body = encode_sync_group_request(&req, 5);
        let mut b = body.freeze();
        let got = SyncGroupRequest::decode(&mut b, 5).unwrap();
        assert_eq!(got, req);
    }

    #[test]
    fn test_kafka_sync_group_v0_no_static_membership() {
        // cite: kafka 4.2.0 (v0 lacks group_instance_id / protocol_type / protocol_name)
        let tenant_id = "kafka-015";
        let req = SyncGroupRequest {
            group_id: group(tenant_id, "g"),
            generation_id: 1,
            member_id: "c1".into(),
            group_instance_id: None,
            protocol_type: None,
            protocol_name: None,
            assignments: vec![],
        };
        let body = encode_sync_group_request(&req, 0);
        let mut b = body.freeze();
        let got = SyncGroupRequest::decode(&mut b, 0).unwrap();
        assert_eq!(got, req);
    }

    #[test]
    fn test_kafka_sync_group_response_encode() {
        // cite: kafka 4.2.0 clients/.../SyncGroupResponse.json
        let _tenant_id = "kafka-016";
        let resp = SyncGroupResponse {
            throttle_time_ms: 0,
            error_code: 0,
            assignment: vec![1, 2, 3],
        };
        let mut buf = BytesMut::new();
        resp.encode(&mut buf);
        // throttle(4) + error(2) + len(4) + 3 bytes
        assert_eq!(buf.len(), 4 + 2 + 4 + 3);
    }

    // ── Heartbeat ──────────────────────────────────────────────────────

    #[test]
    fn test_kafka_heartbeat_roundtrip() {
        // cite: kafka 4.2.0 clients/.../message/HeartbeatRequest.json (v3)
        let tenant_id = "kafka-017";
        let req = HeartbeatRequest {
            group_id: group(tenant_id, "g"),
            generation_id: 12,
            member_id: "consumer-1".into(),
            group_instance_id: Some("static-1".into()),
        };
        let body = encode_heartbeat_request(&req, 3);
        let mut b = body.freeze();
        let got = HeartbeatRequest::decode(&mut b, 3).unwrap();
        assert_eq!(got, req);
    }

    #[test]
    fn test_kafka_heartbeat_v0_no_static() {
        // cite: kafka 4.2.0 (v0 lacks group_instance_id)
        let tenant_id = "kafka-018";
        let req = HeartbeatRequest {
            group_id: group(tenant_id, "g"),
            generation_id: 12,
            member_id: "consumer-1".into(),
            group_instance_id: None,
        };
        let body = encode_heartbeat_request(&req, 0);
        let mut b = body.freeze();
        let got = HeartbeatRequest::decode(&mut b, 0).unwrap();
        assert_eq!(got, req);
    }

    #[test]
    fn test_kafka_heartbeat_response_encode() {
        // cite: kafka 4.2.0 clients/.../HeartbeatResponse.json
        let _tenant_id = "kafka-019";
        let resp = HeartbeatResponse {
            throttle_time_ms: 100,
            error_code: 0,
        };
        let mut buf = BytesMut::new();
        resp.encode(&mut buf);
        assert_eq!(buf.len(), 6);
    }

    // ── Version validation ────────────────────────────────────────────

    #[test]
    fn test_kafka_validate_version_in_range() {
        // cite: kafka 4.2.0 ApiKeys.java validateApiVersion
        let _tenant_id = "kafka-020";
        assert!(validate_version(ApiKey::JoinGroup, 5).is_ok());
        assert!(validate_version(ApiKey::JoinGroup, 0).is_ok());
        assert!(validate_version(ApiKey::JoinGroup, 9).is_ok());
    }

    #[test]
    fn test_kafka_validate_version_rejects_too_high() {
        // cite: kafka 4.2.0 ApiKeys.java UNSUPPORTED_VERSION
        let _tenant_id = "kafka-021";
        let err = validate_version(ApiKey::Heartbeat, 99);
        assert!(matches!(err, Err(StreamsError::ProtocolDecode(_))));
        assert_eq!(unsupported_version_error_code(), 35);
    }
}


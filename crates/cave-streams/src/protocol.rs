// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Kafka wire protocol — API keys, framing, encode/decode primitives.
//!
//! Implements the Kafka binary protocol as defined in the Apache Kafka
//! protocol guide (https://kafka.apache.org/protocol.html).
//!
//! All integers are big-endian. Strings are length-prefixed (INT16 length,
//! followed by UTF-8 bytes; -1 means null). Arrays are INT32-prefixed.

use bytes::{Buf, BufMut, Bytes, BytesMut};

use crate::error::{StreamsError, StreamsResult};

// ── API Key registry ──────────────────────────────────────────────────────────

/// All Kafka API keys supported by CAVE Streams.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i16)]
pub enum ApiKey {
    Produce = 0,
    Fetch = 1,
    ListOffsets = 2,
    Metadata = 3,
    LeaderAndIsr = 4,
    StopReplica = 5,
    UpdateMetadata = 6,
    ControlledShutdown = 7,
    OffsetCommit = 8,
    OffsetFetch = 9,
    FindCoordinator = 10,
    JoinGroup = 11,
    Heartbeat = 12,
    LeaveGroup = 13,
    SyncGroup = 14,
    DescribeGroups = 15,
    ListGroups = 16,
    SaslHandshake = 17,
    ApiVersions = 18,
    CreateTopics = 19,
    DeleteTopics = 20,
    DeleteRecords = 21,
    InitProducerId = 22,
    OffsetForLeaderEpoch = 23,
    AddPartitionsToTxn = 24,
    AddOffsetsToTxn = 25,
    EndTxn = 26,
    WriteTxnMarkers = 27,
    TxnOffsetCommit = 28,
    DescribeAcls = 29,
    CreateAcls = 30,
    DeleteAcls = 31,
    DescribeConfigs = 32,
    AlterConfigs = 33,
    AlterReplicaLogDirs = 34,
    DescribeLogDirs = 35,
    SaslAuthenticate = 36,
    CreatePartitions = 37,
    CreateDelegationToken = 38,
    RenewDelegationToken = 39,
    ExpireDelegationToken = 40,
    DescribeDelegationToken = 41,
    DeleteGroups = 42,
    ElectLeaders = 43,
    IncrementalAlterConfigs = 44,
    AlterPartitionReassignments = 45,
    ListPartitionReassignments = 46,
    OffsetDelete = 47,
    DescribeClientQuotas = 48,
    AlterClientQuotas = 49,
    DescribeUserScramCredentials = 50,
    AlterUserScramCredentials = 51,
    Vote = 52,
    BeginQuorumEpoch = 53,
    EndQuorumEpoch = 54,
    DescribeQuorum = 55,
    AlterPartition = 56,
    UpdateFeatures = 57,
    Envelope = 58,
    FetchSnapshot = 59,
    DescribeCluster = 60,
    DescribeProducers = 61,
    UnregisterBroker = 64,
    DescribeTransactions = 65,
    ListTransactions = 66,
    AllocateProducerIds = 67,
}

impl ApiKey {
    pub fn from_i16(v: i16) -> Option<Self> {
        Some(match v {
            0 => Self::Produce,
            1 => Self::Fetch,
            2 => Self::ListOffsets,
            3 => Self::Metadata,
            4 => Self::LeaderAndIsr,
            5 => Self::StopReplica,
            6 => Self::UpdateMetadata,
            7 => Self::ControlledShutdown,
            8 => Self::OffsetCommit,
            9 => Self::OffsetFetch,
            10 => Self::FindCoordinator,
            11 => Self::JoinGroup,
            12 => Self::Heartbeat,
            13 => Self::LeaveGroup,
            14 => Self::SyncGroup,
            15 => Self::DescribeGroups,
            16 => Self::ListGroups,
            17 => Self::SaslHandshake,
            18 => Self::ApiVersions,
            19 => Self::CreateTopics,
            20 => Self::DeleteTopics,
            21 => Self::DeleteRecords,
            22 => Self::InitProducerId,
            23 => Self::OffsetForLeaderEpoch,
            24 => Self::AddPartitionsToTxn,
            25 => Self::AddOffsetsToTxn,
            26 => Self::EndTxn,
            27 => Self::WriteTxnMarkers,
            28 => Self::TxnOffsetCommit,
            29 => Self::DescribeAcls,
            30 => Self::CreateAcls,
            31 => Self::DeleteAcls,
            32 => Self::DescribeConfigs,
            33 => Self::AlterConfigs,
            34 => Self::AlterReplicaLogDirs,
            35 => Self::DescribeLogDirs,
            36 => Self::SaslAuthenticate,
            37 => Self::CreatePartitions,
            38 => Self::CreateDelegationToken,
            39 => Self::RenewDelegationToken,
            40 => Self::ExpireDelegationToken,
            41 => Self::DescribeDelegationToken,
            42 => Self::DeleteGroups,
            43 => Self::ElectLeaders,
            44 => Self::IncrementalAlterConfigs,
            45 => Self::AlterPartitionReassignments,
            46 => Self::ListPartitionReassignments,
            47 => Self::OffsetDelete,
            48 => Self::DescribeClientQuotas,
            49 => Self::AlterClientQuotas,
            50 => Self::DescribeUserScramCredentials,
            51 => Self::AlterUserScramCredentials,
            52 => Self::Vote,
            53 => Self::BeginQuorumEpoch,
            54 => Self::EndQuorumEpoch,
            55 => Self::DescribeQuorum,
            56 => Self::AlterPartition,
            57 => Self::UpdateFeatures,
            58 => Self::Envelope,
            59 => Self::FetchSnapshot,
            60 => Self::DescribeCluster,
            61 => Self::DescribeProducers,
            64 => Self::UnregisterBroker,
            65 => Self::DescribeTransactions,
            66 => Self::ListTransactions,
            67 => Self::AllocateProducerIds,
            _ => return None,
        })
    }

    /// Minimum and maximum supported versions for each API key.
    pub fn version_range(self) -> (i16, i16) {
        match self {
            Self::Produce => (0, 9),
            Self::Fetch => (0, 15),
            Self::ListOffsets => (0, 7),
            Self::Metadata => (0, 12),
            Self::OffsetCommit => (0, 9),
            Self::OffsetFetch => (0, 8),
            Self::FindCoordinator => (0, 4),
            Self::JoinGroup => (0, 9),
            Self::Heartbeat => (0, 4),
            Self::LeaveGroup => (0, 5),
            Self::SyncGroup => (0, 5),
            Self::DescribeGroups => (0, 5),
            Self::ListGroups => (0, 4),
            Self::ApiVersions => (0, 3),
            Self::CreateTopics => (0, 7),
            Self::DeleteTopics => (0, 6),
            Self::DeleteRecords => (0, 2),
            Self::InitProducerId => (0, 4),
            Self::AddPartitionsToTxn => (0, 3),
            Self::AddOffsetsToTxn => (0, 3),
            Self::EndTxn => (0, 3),
            Self::TxnOffsetCommit => (0, 3),
            Self::DescribeConfigs => (0, 4),
            Self::AlterConfigs => (0, 2),
            Self::IncrementalAlterConfigs => (0, 1),
            Self::CreatePartitions => (0, 3),
            Self::DeleteGroups => (0, 2),
            Self::AlterPartitionReassignments => (0, 0),
            Self::ListPartitionReassignments => (0, 0),
            Self::DescribeProducers => (0, 0),
            Self::DescribeAcls => (0, 3),
            Self::CreateAcls => (0, 3),
            Self::DeleteAcls => (0, 3),
            Self::DescribeClientQuotas => (0, 1),
            Self::AlterClientQuotas => (0, 1),
            // KIP-595 KRaft RPCs — cave-streams implements v0
            // for each (the bare contract every voter ships).
            Self::Vote => (0, 0),
            Self::BeginQuorumEpoch => (0, 0),
            Self::EndQuorumEpoch => (0, 0),
            Self::DescribeQuorum => (0, 1),
            Self::FetchSnapshot => (0, 0),
            _ => (0, 0),
        }
    }
}

// ── Request header ─────────────────────────────────────────────────────────────

/// Decoded Kafka request header (before the API-specific body).
#[derive(Debug, Clone)]
pub struct RequestHeader {
    pub api_key: ApiKey,
    pub api_version: i16,
    pub correlation_id: i32,
    pub client_id: Option<String>,
}

impl RequestHeader {
    /// Decode a request header from the beginning of a framed request buffer.
    /// Advances `buf` past the header bytes.
    pub fn decode(buf: &mut impl Buf) -> StreamsResult<Self> {
        if buf.remaining() < 8 {
            return Err(StreamsError::ProtocolDecode(
                "request header too short".into(),
            ));
        }
        let api_key_raw = buf.get_i16();
        let api_version = buf.get_i16();
        let correlation_id = buf.get_i32();

        let api_key = ApiKey::from_i16(api_key_raw).ok_or_else(|| {
            StreamsError::ProtocolDecode(format!("unknown api_key: {api_key_raw}"))
        })?;

        let client_id = decode_nullable_string(buf)?;

        Ok(Self {
            api_key,
            api_version,
            correlation_id,
            client_id,
        })
    }
}

// ── Primitive encode/decode helpers ──────────────────────────────────────────

/// Decode a nullable string: INT16 length (–1 = null) + UTF-8 bytes.
pub fn decode_nullable_string(buf: &mut dyn Buf) -> StreamsResult<Option<String>> {
    if buf.remaining() < 2 {
        return Err(StreamsError::ProtocolDecode("expected string length".into()));
    }
    let len = buf.get_i16();
    if len == -1 {
        return Ok(None);
    }
    let len = len as usize;
    if buf.remaining() < len {
        return Err(StreamsError::ProtocolDecode(format!(
            "string body truncated: need {len}, have {}",
            buf.remaining()
        )));
    }
    let bytes = buf.copy_to_bytes(len);
    Ok(Some(
        String::from_utf8(bytes.to_vec())
            .map_err(|e| StreamsError::ProtocolDecode(e.to_string()))?,
    ))
}

/// Decode a mandatory string (length must be ≥ 0).
pub fn decode_string(buf: &mut dyn Buf) -> StreamsResult<String> {
    decode_nullable_string(buf)?.ok_or_else(|| StreamsError::ProtocolDecode("unexpected null string".into()))
}

/// Decode an INT32-prefixed array, calling `item_fn` for each element.
pub fn decode_array<T, F>(buf: &mut dyn Buf, item_fn: F) -> StreamsResult<Vec<T>>
where
    F: Fn(&mut dyn Buf) -> StreamsResult<T>,
{
    if buf.remaining() < 4 {
        return Err(StreamsError::ProtocolDecode("expected array length".into()));
    }
    let count = buf.get_i32();
    if count == -1 {
        return Ok(vec![]);
    }
    let count = count as usize;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        out.push(item_fn(buf)?);
    }
    Ok(out)
}

/// Encode a nullable string.
pub fn encode_nullable_string(buf: &mut BytesMut, s: Option<&str>) {
    match s {
        None => buf.put_i16(-1),
        Some(v) => {
            buf.put_i16(v.len() as i16);
            buf.put_slice(v.as_bytes());
        }
    }
}

/// Encode a mandatory string.
pub fn encode_string(buf: &mut BytesMut, s: &str) {
    buf.put_i16(s.len() as i16);
    buf.put_slice(s.as_bytes());
}

/// Encode an array with a length prefix.
pub fn encode_array<T, F>(buf: &mut BytesMut, items: &[T], item_fn: F)
where
    F: Fn(&mut BytesMut, &T),
{
    buf.put_i32(items.len() as i32);
    for item in items {
        item_fn(buf, item);
    }
}

/// Wrap a pre-encoded response body with the standard response framing:
/// [INT32 total_length] [INT32 correlation_id] [body].
pub fn frame_response(correlation_id: i32, body: Bytes) -> Bytes {
    let total_len = 4 + body.len(); // 4 bytes for correlation_id + body
    let mut out = BytesMut::with_capacity(4 + total_len);
    out.put_i32(total_len as i32);
    out.put_i32(correlation_id);
    out.put_slice(&body);
    out.freeze()
}

// ── Produce request/response ──────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ProduceRequest {
    pub transactional_id: Option<String>,
    pub acks: i16,
    pub timeout_ms: i32,
    pub topic_data: Vec<ProduceTopicData>,
}

#[derive(Debug, Clone)]
pub struct ProduceTopicData {
    pub name: String,
    pub partition_data: Vec<ProducePartitionData>,
}

#[derive(Debug, Clone)]
pub struct ProducePartitionData {
    pub index: i32,
    pub records: Bytes,
}

impl ProduceRequest {
    pub fn decode(buf: &mut impl Buf, _version: i16) -> StreamsResult<Self> {
        let transactional_id = decode_nullable_string(buf)?;
        let acks = buf.get_i16();
        let timeout_ms = buf.get_i32();
        let topic_data = decode_array(buf, |b| {
            let name = decode_string(b)?;
            let partition_data = decode_array(b, |pb| {
                let index = pb.get_i32();
                let record_len = pb.get_i32();
                let records = if record_len > 0 {
                    pb.copy_to_bytes(record_len as usize)
                } else {
                    Bytes::new()
                };
                Ok(ProducePartitionData { index, records })
            })?;
            Ok(ProduceTopicData { name, partition_data })
        })?;
        Ok(Self {
            transactional_id,
            acks,
            timeout_ms,
            topic_data,
        })
    }
}

#[derive(Debug, Clone)]
pub struct ProduceResponse {
    pub responses: Vec<ProduceTopicResponse>,
    pub throttle_time_ms: i32,
}

#[derive(Debug, Clone)]
pub struct ProduceTopicResponse {
    pub name: String,
    pub partition_responses: Vec<ProducePartitionResponse>,
}

#[derive(Debug, Clone)]
pub struct ProducePartitionResponse {
    pub index: i32,
    pub error_code: i16,
    pub base_offset: i64,
    pub log_append_time_ms: i64,
    pub log_start_offset: i64,
}

impl ProduceResponse {
    pub fn encode(&self, buf: &mut BytesMut) {
        encode_array(buf, &self.responses, |b, r| {
            encode_string(b, &r.name);
            encode_array(b, &r.partition_responses, |pb, pr| {
                pb.put_i32(pr.index);
                pb.put_i16(pr.error_code);
                pb.put_i64(pr.base_offset);
                pb.put_i64(pr.log_append_time_ms);
                pb.put_i64(pr.log_start_offset);
            });
        });
        buf.put_i32(self.throttle_time_ms);
    }
}

// ── Fetch request/response ────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct FetchRequest {
    pub replica_id: i32,
    pub max_wait_ms: i32,
    pub min_bytes: i32,
    pub max_bytes: i32,
    pub isolation_level: i8,
    pub topics: Vec<FetchTopic>,
}

#[derive(Debug, Clone)]
pub struct FetchTopic {
    pub name: String,
    pub partitions: Vec<FetchPartition>,
}

#[derive(Debug, Clone)]
pub struct FetchPartition {
    pub partition: i32,
    pub current_leader_epoch: i32,
    pub fetch_offset: i64,
    pub partition_max_bytes: i32,
}

impl FetchRequest {
    pub fn decode(buf: &mut impl Buf, version: i16) -> StreamsResult<Self> {
        let replica_id = buf.get_i32();
        let max_wait_ms = buf.get_i32();
        let min_bytes = buf.get_i32();
        let max_bytes = if version >= 3 { buf.get_i32() } else { i32::MAX };
        let isolation_level = if version >= 4 { buf.get_i8() } else { 0 };
        let topics = decode_array(buf, |b| {
            let name = decode_string(b)?;
            let partitions = decode_array(b, |pb| {
                let partition = pb.get_i32();
                let current_leader_epoch = if version >= 9 { pb.get_i32() } else { -1 };
                let fetch_offset = pb.get_i64();
                let partition_max_bytes = pb.get_i32();
                Ok(FetchPartition { partition, current_leader_epoch, fetch_offset, partition_max_bytes })
            })?;
            Ok(FetchTopic { name, partitions })
        })?;
        Ok(Self { replica_id, max_wait_ms, min_bytes, max_bytes, isolation_level, topics })
    }
}

// ── Metadata request/response ─────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct MetadataRequest {
    pub topics: Option<Vec<String>>,
    pub allow_auto_topic_creation: bool,
}

impl MetadataRequest {
    pub fn decode(buf: &mut impl Buf, version: i16) -> StreamsResult<Self> {
        let topics_raw = decode_array(buf, |b| decode_string(b))?;
        let topics = if topics_raw.is_empty() { None } else { Some(topics_raw) };
        let allow_auto_topic_creation = if version >= 4 { buf.get_u8() != 0 } else { true };
        Ok(Self { topics, allow_auto_topic_creation })
    }
}

// ── CreateTopics request/response ─────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct CreateTopicsRequest {
    pub topics: Vec<CreateTopicConfig>,
    pub timeout_ms: i32,
    pub validate_only: bool,
}

#[derive(Debug, Clone)]
pub struct CreateTopicConfig {
    pub name: String,
    pub num_partitions: i32,
    pub replication_factor: i16,
    pub configs: Vec<(String, Option<String>)>,
}

impl CreateTopicsRequest {
    pub fn decode(buf: &mut impl Buf, version: i16) -> StreamsResult<Self> {
        let topics = decode_array(buf, |b| {
            let name = decode_string(b)?;
            let num_partitions = b.get_i32();
            let replication_factor = b.get_i16();
            // Skip assignments (INT32-prefixed, each has INT32 partition + INT32[] replicas)
            let _assignments = decode_array(b, |ab| {
                let _part = ab.get_i32();
                let _replicas = decode_array(ab, |rb| Ok(rb.get_i32()))?;
                Ok(())
            })?;
            let configs = decode_array(b, |cb| {
                let k = decode_string(cb)?;
                let v = decode_nullable_string(cb)?;
                Ok((k, v))
            })?;
            Ok(CreateTopicConfig { name, num_partitions, replication_factor, configs })
        })?;
        let timeout_ms = buf.get_i32();
        let validate_only = if version >= 1 { buf.get_u8() != 0 } else { false };
        Ok(Self { topics, timeout_ms, validate_only })
    }
}

// ── ApiVersions response ──────────────────────────────────────────────────────

/// Build an ApiVersions response listing all supported APIs.
pub fn build_api_versions_response() -> BytesMut {
    let supported: &[(ApiKey, i16, i16)] = &[
        (ApiKey::Produce, 0, 9),
        (ApiKey::Fetch, 0, 15),
        (ApiKey::ListOffsets, 0, 7),
        (ApiKey::Metadata, 0, 12),
        (ApiKey::OffsetCommit, 0, 9),
        (ApiKey::OffsetFetch, 0, 8),
        (ApiKey::FindCoordinator, 0, 4),
        (ApiKey::JoinGroup, 0, 9),
        (ApiKey::Heartbeat, 0, 4),
        (ApiKey::LeaveGroup, 0, 5),
        (ApiKey::SyncGroup, 0, 5),
        (ApiKey::DescribeGroups, 0, 5),
        (ApiKey::ListGroups, 0, 4),
        (ApiKey::ApiVersions, 0, 3),
        (ApiKey::CreateTopics, 0, 7),
        (ApiKey::DeleteTopics, 0, 6),
        (ApiKey::DeleteRecords, 0, 2),
        (ApiKey::InitProducerId, 0, 4),
        (ApiKey::AddPartitionsToTxn, 0, 3),
        (ApiKey::AddOffsetsToTxn, 0, 3),
        (ApiKey::EndTxn, 0, 3),
        (ApiKey::TxnOffsetCommit, 0, 3),
        (ApiKey::DescribeConfigs, 0, 4),
        (ApiKey::AlterConfigs, 0, 2),
        (ApiKey::IncrementalAlterConfigs, 0, 1),
        (ApiKey::CreatePartitions, 0, 3),
        (ApiKey::DeleteGroups, 0, 2),
        (ApiKey::AlterPartitionReassignments, 0, 0),
        (ApiKey::ListPartitionReassignments, 0, 0),
        (ApiKey::DescribeProducers, 0, 0),
        (ApiKey::DescribeAcls, 0, 3),
        (ApiKey::CreateAcls, 0, 3),
        (ApiKey::DeleteAcls, 0, 3),
        (ApiKey::DescribeClientQuotas, 0, 1),
        (ApiKey::AlterClientQuotas, 0, 1),
    ];

    let mut buf = BytesMut::new();
    buf.put_i16(0); // error_code
    buf.put_i32(supported.len() as i32);
    for (key, min, max) in supported {
        buf.put_i16(*key as i16);
        buf.put_i16(*min);
        buf.put_i16(*max);
    }
    buf.put_i32(0); // throttle_time_ms
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_api_key_roundtrip() {
        for v in [0i16, 1, 2, 3, 8, 9, 11, 19, 22, 44, 61, 67] {
            let key = ApiKey::from_i16(v);
            assert!(key.is_some(), "api_key {v} should be recognised");
        }
        assert!(ApiKey::from_i16(100).is_none());
    }

    #[test]
    fn test_encode_decode_string() {
        let mut buf = BytesMut::new();
        encode_string(&mut buf, "hello-kafka");
        let mut b = buf.freeze();
        let s = decode_string(&mut b).unwrap();
        assert_eq!(s, "hello-kafka");
    }

    #[test]
    fn test_frame_response() {
        let body = Bytes::from_static(b"payload");
        let framed = frame_response(42, body.clone());
        // First 4 bytes = total_len = 4 (correlation) + 7 (payload) = 11
        assert_eq!(&framed[..4], &11i32.to_be_bytes());
        // Next 4 bytes = correlation_id = 42
        assert_eq!(&framed[4..8], &42i32.to_be_bytes());
        // Remainder = payload
        assert_eq!(&framed[8..], b"payload");
    }

    #[test]
    fn test_api_versions_response_non_empty() {
        let resp = build_api_versions_response();
        assert!(!resp.is_empty());
        // error_code = 0 (first 2 bytes)
        assert_eq!(resp[0..2], [0, 0]);
    }
}

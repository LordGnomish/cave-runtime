// SPDX-License-Identifier: AGPL-3.0-or-later
//! Kafka-compatible wire protocol implementation.
//!
//! Implements enough of the Kafka binary protocol for existing Kafka clients
//! (producers, consumers, admin tools) to connect without modification.
//!
//! Supported API keys:
//!   0  – Produce
//!   1  – Fetch
//!   2  – ListOffsets
//!   3  – Metadata
//!   8  – OffsetCommit
//!   9  – OffsetFetch
//!  10  – FindCoordinator
//!  11  – JoinGroup
//!  12  – Heartbeat
//!  13  – LeaveGroup
//!  14  – SyncGroup
//!  18  – ApiVersions
//!
//! Wire format: big-endian, 4-byte message length prefix.

use crate::error::{StreamError, StreamResult};
use crate::models::{
    PartitionerStrategy, ProducerRecord, Record, TopicPartition,
    RebalanceProtocol,
};
use crate::storage::StreamStorage;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tracing::{debug, error, info, warn};

// ─── API key constants ────────────────────────────────────────────────────────

pub const API_PRODUCE: i16 = 0;
pub const API_FETCH: i16 = 1;
pub const API_LIST_OFFSETS: i16 = 2;
pub const API_METADATA: i16 = 3;
pub const API_OFFSET_COMMIT: i16 = 8;
pub const API_OFFSET_FETCH: i16 = 9;
pub const API_FIND_COORDINATOR: i16 = 10;
pub const API_JOIN_GROUP: i16 = 11;
pub const API_HEARTBEAT: i16 = 12;
pub const API_LEAVE_GROUP: i16 = 13;
pub const API_SYNC_GROUP: i16 = 14;
pub const API_API_VERSIONS: i16 = 18;

// ─── Error codes ──────────────────────────────────────────────────────────────

pub const ERR_NONE: i16 = 0;
pub const ERR_UNKNOWN_TOPIC: i16 = 3;
pub const ERR_LEADER_NOT_AVAILABLE: i16 = 5;
pub const ERR_NOT_LEADER: i16 = 6;
pub const ERR_OFFSET_OUT_OF_RANGE: i16 = 1;
pub const ERR_COORDINATOR_NOT_AVAILABLE: i16 = 15;
pub const ERR_REBALANCE_IN_PROGRESS: i16 = 27;
pub const ERR_UNKNOWN_MEMBER: i16 = 25;
pub const ERR_ILLEGAL_GENERATION: i16 = 22;

// ─── Request / response types ─────────────────────────────────────────────────

/// Decoded request header (common to all Kafka requests).
#[derive(Debug)]
pub struct RequestHeader {
    pub api_key: i16,
    pub api_version: i16,
    pub correlation_id: i32,
    pub client_id: String,
}

/// Raw Kafka frame (header + body bytes).
#[derive(Debug)]
pub struct KafkaRequest {
    pub header: RequestHeader,
    /// Remaining bytes after the header (body is API-specific).
    pub body: Vec<u8>,
}

// ─── TCP server ───────────────────────────────────────────────────────────────

/// Kafka-compatible TCP listener.
///
/// Bind the listener with [`KafkaServer::bind`], then call
/// [`KafkaServer::run`] inside a Tokio task to accept connections.
pub struct KafkaServer<S: StreamStorage + Clone + 'static> {
    storage: S,
    addr: String,
}

impl<S: StreamStorage + Clone + 'static> KafkaServer<S> {
    pub fn new(storage: S, addr: impl Into<String>) -> Self {
        Self {
            storage,
            addr: addr.into(),
        }
    }

    /// Run the TCP server (blocks until the listener errors).
    pub async fn run(self) -> StreamResult<()> {
        let listener = TcpListener::bind(&self.addr)
            .await
            .map_err(|e| StreamError::Protocol(format!("Bind failed on {}: {e}", self.addr)))?;

        info!(addr = %self.addr, "Kafka-compatible listener started");

        loop {
            let (stream, peer) = listener
                .accept()
                .await
                .map_err(|e| StreamError::Protocol(format!("Accept error: {e}")))?;

            debug!(%peer, "New Kafka client connection");

            let storage = self.storage.clone();
            tokio::spawn(async move {
                if let Err(e) = handle_connection(stream, storage).await {
                    warn!(%peer, err = %e, "Connection error");
                }
            });
        }
    }
}

// ─── Connection handler ───────────────────────────────────────────────────────

async fn handle_connection<S: StreamStorage>(
    mut stream: TcpStream,
    storage: S,
) -> StreamResult<()> {
    loop {
        // Read 4-byte message length.
        let mut len_buf = [0u8; 4];
        match stream.read_exact(&mut len_buf).await {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(StreamError::Protocol(format!("Read error: {e}"))),
        }
        let msg_len = i32::from_be_bytes(len_buf) as usize;
        if msg_len == 0 {
            break;
        }

        let mut msg_buf = vec![0u8; msg_len];
        stream
            .read_exact(&mut msg_buf)
            .await
            .map_err(|e| StreamError::Protocol(format!("Read body error: {e}")))?;

        let request = decode_request(&msg_buf)?;
        debug!(api_key = request.header.api_key, "Kafka request");

        let response_body = dispatch(&request, &storage)?;

        // Write response: 4-byte correlation_id + body.
        let mut response = Vec::with_capacity(4 + 4 + response_body.len());
        write_i32(&mut response, (4 + response_body.len()) as i32);
        write_i32(&mut response, request.header.correlation_id);
        response.extend_from_slice(&response_body);

        stream
            .write_all(&response)
            .await
            .map_err(|e| StreamError::Protocol(format!("Write error: {e}")))?;
    }
    Ok(())
}

// ─── Request decoder ─────────────────────────────────────────────────────────

fn decode_request(buf: &[u8]) -> StreamResult<KafkaRequest> {
    let mut cur = Cursor::new(buf);

    let api_key = cur.read_i16()?;
    let api_version = cur.read_i16()?;
    let correlation_id = cur.read_i32()?;
    let client_id = cur.read_string()?;

    Ok(KafkaRequest {
        header: RequestHeader {
            api_key,
            api_version,
            correlation_id,
            client_id,
        },
        body: cur.remaining().to_vec(),
    })
}

// ─── Dispatcher ──────────────────────────────────────────────────────────────

fn dispatch<S: StreamStorage>(
    req: &KafkaRequest,
    storage: &S,
) -> StreamResult<Vec<u8>> {
    match req.header.api_key {
        API_API_VERSIONS => handle_api_versions(req),
        API_METADATA => handle_metadata(req, storage),
        API_PRODUCE => handle_produce(req, storage),
        API_FETCH => handle_fetch(req, storage),
        API_LIST_OFFSETS => handle_list_offsets(req, storage),
        API_OFFSET_COMMIT => handle_offset_commit(req, storage),
        API_OFFSET_FETCH => handle_offset_fetch(req, storage),
        API_FIND_COORDINATOR => handle_find_coordinator(req),
        API_JOIN_GROUP => handle_join_group(req, storage),
        API_SYNC_GROUP => handle_sync_group(req, storage),
        API_HEARTBEAT => handle_heartbeat(req, storage),
        API_LEAVE_GROUP => handle_leave_group(req, storage),
        other => {
            warn!(api_key = other, "Unsupported Kafka API");
            // Return empty error response.
            let mut body = Vec::new();
            write_i16(&mut body, 35); // UNSUPPORTED_VERSION
            Ok(body)
        }
    }
}

// ─── ApiVersions (18) ────────────────────────────────────────────────────────

fn handle_api_versions(_req: &KafkaRequest) -> StreamResult<Vec<u8>> {
    let supported: &[(i16, i16, i16)] = &[
        (API_PRODUCE, 0, 8),
        (API_FETCH, 0, 12),
        (API_LIST_OFFSETS, 0, 5),
        (API_METADATA, 0, 9),
        (API_OFFSET_COMMIT, 0, 8),
        (API_OFFSET_FETCH, 0, 7),
        (API_FIND_COORDINATOR, 0, 3),
        (API_JOIN_GROUP, 0, 7),
        (API_HEARTBEAT, 0, 4),
        (API_LEAVE_GROUP, 0, 4),
        (API_SYNC_GROUP, 0, 5),
        (API_API_VERSIONS, 0, 3),
    ];

    let mut body = Vec::new();
    write_i16(&mut body, ERR_NONE);
    write_i32(&mut body, supported.len() as i32);
    for &(key, min_ver, max_ver) in supported {
        write_i16(&mut body, key);
        write_i16(&mut body, min_ver);
        write_i16(&mut body, max_ver);
    }
    // throttle_time_ms
    write_i32(&mut body, 0);
    Ok(body)
}

// ─── Metadata (3) ────────────────────────────────────────────────────────────

fn handle_metadata<S: StreamStorage>(req: &KafkaRequest, storage: &S) -> StreamResult<Vec<u8>> {
    let mut cur = Cursor::new(&req.body);
    let topic_count = cur.read_i32().unwrap_or(0);
    let mut requested: Vec<String> = Vec::new();
    for _ in 0..topic_count {
        if let Ok(t) = cur.read_string() {
            requested.push(t);
        }
    }

    // Resolve topics to return.
    let topics: Vec<crate::models::TopicInfo> = if requested.is_empty() {
        storage.list_topics().unwrap_or_default()
    } else {
        requested
            .iter()
            .filter_map(|name| storage.get_topic(name).ok().flatten())
            .collect()
    };

    let mut body = Vec::new();
    // throttle_time_ms
    write_i32(&mut body, 0);
    // brokers array (single broker: us)
    write_i32(&mut body, 1);
    write_i32(&mut body, 1); // node_id
    write_string(&mut body, "localhost");
    write_i32(&mut body, 9092); // port
    write_string(&mut body, ""); // rack

    // cluster_id
    write_string(&mut body, "cave-streams-cluster");
    // controller_id
    write_i32(&mut body, 1);

    // topics
    write_i32(&mut body, topics.len() as i32);
    for topic in &topics {
        write_i16(&mut body, ERR_NONE);
        write_string(&mut body, &topic.name);
        write_bool(&mut body, false); // is_internal

        // partitions
        write_i32(&mut body, topic.partitions as i32);
        for p in 0..topic.partitions {
            write_i16(&mut body, ERR_NONE);
            write_i32(&mut body, p as i32); // partition_index
            write_i32(&mut body, 1); // leader_id
            write_i32(&mut body, 0); // leader_epoch
            write_i32(&mut body, 1); // replica count
            write_i32(&mut body, 1); // replica[0] = broker 1
            write_i32(&mut body, 1); // isr count
            write_i32(&mut body, 1); // isr[0]
            write_i32(&mut body, 0); // offline replicas
        }
    }
    Ok(body)
}

// ─── Produce (0) ─────────────────────────────────────────────────────────────

fn handle_produce<S: StreamStorage>(req: &KafkaRequest, storage: &S) -> StreamResult<Vec<u8>> {
    let mut cur = Cursor::new(&req.body);
    let _transactional_id = cur.read_nullable_string().unwrap_or_default();
    let _acks = cur.read_i16().unwrap_or(1);
    let _timeout_ms = cur.read_i32().unwrap_or(30_000);
    let topic_count = cur.read_i32().unwrap_or(0);

    let mut body = Vec::new();
    write_i32(&mut body, topic_count);

    for _ in 0..topic_count {
        let topic_name = cur.read_string().unwrap_or_default();
        let partition_count = cur.read_i32().unwrap_or(0);

        write_string(&mut body, &topic_name);
        write_i32(&mut body, partition_count);

        for _ in 0..partition_count {
            let partition = cur.read_i32().unwrap_or(0) as u32;
            let _record_set_size = cur.read_i32().unwrap_or(0);
            // Decode a simplified record batch.
            let key = cur.read_bytes().unwrap_or_default();
            let value = cur.read_bytes().unwrap_or_default();

            let record = Record::new(
                &topic_name,
                partition,
                if key.is_empty() { None } else { Some(key) },
                if value.is_empty() { None } else { Some(value) },
            );

            let offset = storage
                .append_to_partition(&topic_name, partition, record)
                .unwrap_or(-1);

            write_i32(&mut body, partition as i32);
            write_i16(&mut body, ERR_NONE);
            write_i64(&mut body, offset);
            write_i64(&mut body, chrono::Utc::now().timestamp_millis()); // log_append_time
            write_i32(&mut body, 0); // log_start_offset (simplified)
        }
    }
    // throttle_time_ms
    write_i32(&mut body, 0);
    Ok(body)
}

// ─── Fetch (1) ───────────────────────────────────────────────────────────────

fn handle_fetch<S: StreamStorage>(req: &KafkaRequest, storage: &S) -> StreamResult<Vec<u8>> {
    let mut cur = Cursor::new(&req.body);
    let _replica_id = cur.read_i32().unwrap_or(-1);
    let _max_wait_ms = cur.read_i32().unwrap_or(500);
    let _min_bytes = cur.read_i32().unwrap_or(1);
    let _max_bytes = cur.read_i32().unwrap_or(52_428_800);
    let _isolation_level = cur.read_i8().unwrap_or(0);
    let topic_count = cur.read_i32().unwrap_or(0);

    let mut body = Vec::new();
    write_i32(&mut body, 0); // throttle_time_ms
    write_i16(&mut body, ERR_NONE); // error_code
    write_i32(&mut body, 0); // session_id
    write_i32(&mut body, topic_count);

    for _ in 0..topic_count {
        let topic_name = cur.read_string().unwrap_or_default();
        let partition_count = cur.read_i32().unwrap_or(0);

        write_string(&mut body, &topic_name);
        write_i32(&mut body, partition_count);

        for _ in 0..partition_count {
            let partition = cur.read_i32().unwrap_or(0) as u32;
            let fetch_offset = cur.read_i64().unwrap_or(0);
            let _log_start_offset = cur.read_i64().unwrap_or(0);
            let _max_partition_bytes = cur.read_i32().unwrap_or(1_048_576);

            let records = storage
                .fetch_from_partition(&topic_name, partition, fetch_offset, 100)
                .unwrap_or_default();

            let hwm = storage.high_watermark(&topic_name, partition).unwrap_or(0);

            write_i32(&mut body, partition as i32);
            write_i16(&mut body, ERR_NONE);
            write_i64(&mut body, hwm); // high_watermark
            write_i64(&mut body, storage.log_start_offset(&topic_name, partition).unwrap_or(0));
            write_i32(&mut body, -1); // preferred_read_replica

            // Encode records as a simplified record batch.
            let mut record_bytes = Vec::new();
            for r in &records {
                write_i64(&mut record_bytes, r.offset);
                write_i64(&mut record_bytes, r.timestamp_ms);
                write_bytes(
                    &mut record_bytes,
                    r.key.as_deref().unwrap_or(&[]),
                );
                write_bytes(
                    &mut record_bytes,
                    r.value.as_deref().unwrap_or(&[]),
                );
            }
            write_i32(&mut body, record_bytes.len() as i32);
            body.extend_from_slice(&record_bytes);
        }
    }
    Ok(body)
}

// ─── ListOffsets (2) ─────────────────────────────────────────────────────────

fn handle_list_offsets<S: StreamStorage>(
    req: &KafkaRequest,
    storage: &S,
) -> StreamResult<Vec<u8>> {
    let mut cur = Cursor::new(&req.body);
    let _replica_id = cur.read_i32().unwrap_or(-1);
    let _isolation_level = cur.read_i8().unwrap_or(0);
    let topic_count = cur.read_i32().unwrap_or(0);

    let mut body = Vec::new();
    write_i32(&mut body, 0); // throttle_time_ms
    write_i32(&mut body, topic_count);

    for _ in 0..topic_count {
        let topic_name = cur.read_string().unwrap_or_default();
        let partition_count = cur.read_i32().unwrap_or(0);
        write_string(&mut body, &topic_name);
        write_i32(&mut body, partition_count);

        for _ in 0..partition_count {
            let partition = cur.read_i32().unwrap_or(0) as u32;
            let timestamp = cur.read_i64().unwrap_or(-1); // -1 = latest, -2 = earliest

            let offset = if timestamp == -2 {
                storage.log_start_offset(&topic_name, partition).unwrap_or(0)
            } else {
                storage.high_watermark(&topic_name, partition).unwrap_or(0)
            };

            write_i32(&mut body, partition as i32);
            write_i16(&mut body, ERR_NONE);
            write_i64(&mut body, chrono::Utc::now().timestamp_millis()); // timestamp
            write_i64(&mut body, offset);
        }
    }
    Ok(body)
}

// ─── OffsetCommit (8) ────────────────────────────────────────────────────────

fn handle_offset_commit<S: StreamStorage>(
    req: &KafkaRequest,
    storage: &S,
) -> StreamResult<Vec<u8>> {
    let mut cur = Cursor::new(&req.body);
    let group_id = cur.read_string().unwrap_or_default();
    let _generation = cur.read_i32().unwrap_or(-1);
    let _member_id = cur.read_string().unwrap_or_default();
    let topic_count = cur.read_i32().unwrap_or(0);

    let mut body = Vec::new();
    write_i32(&mut body, 0); // throttle_time_ms
    write_i32(&mut body, topic_count);

    for _ in 0..topic_count {
        let topic_name = cur.read_string().unwrap_or_default();
        let partition_count = cur.read_i32().unwrap_or(0);
        write_string(&mut body, &topic_name);
        write_i32(&mut body, partition_count);

        for _ in 0..partition_count {
            let partition = cur.read_i32().unwrap_or(0) as u32;
            let offset = cur.read_i64().unwrap_or(0);
            let _metadata = cur.read_nullable_string().unwrap_or_default();

            let err = match storage.commit_offset(&group_id, &topic_name, partition, offset) {
                Ok(_) => ERR_NONE,
                Err(_) => 39, // GROUP_AUTHORIZATION_FAILED (fallback)
            };

            write_i32(&mut body, partition as i32);
            write_i16(&mut body, err);
        }
    }
    Ok(body)
}

// ─── OffsetFetch (9) ─────────────────────────────────────────────────────────

fn handle_offset_fetch<S: StreamStorage>(
    req: &KafkaRequest,
    storage: &S,
) -> StreamResult<Vec<u8>> {
    let mut cur = Cursor::new(&req.body);
    let group_id = cur.read_string().unwrap_or_default();
    let topic_count = cur.read_i32().unwrap_or(0);

    let mut body = Vec::new();
    write_i32(&mut body, 0); // throttle_time_ms
    write_i32(&mut body, topic_count);

    for _ in 0..topic_count {
        let topic_name = cur.read_string().unwrap_or_default();
        let partition_count = cur.read_i32().unwrap_or(0);
        write_string(&mut body, &topic_name);
        write_i32(&mut body, partition_count);

        for _ in 0..partition_count {
            let partition = cur.read_i32().unwrap_or(0) as u32;
            let offset = storage
                .get_offset(&group_id, &topic_name, partition)
                .unwrap_or(0);

            write_i32(&mut body, partition as i32);
            write_i64(&mut body, offset);
            write_string(&mut body, ""); // metadata
            write_i16(&mut body, ERR_NONE);
        }
    }
    write_i16(&mut body, ERR_NONE); // error_code
    Ok(body)
}

// ─── FindCoordinator (10) ────────────────────────────────────────────────────

fn handle_find_coordinator(_req: &KafkaRequest) -> StreamResult<Vec<u8>> {
    let mut body = Vec::new();
    write_i32(&mut body, 0); // throttle_time_ms
    write_i16(&mut body, ERR_NONE);
    write_string(&mut body, ""); // error_message
    write_i32(&mut body, 1); // node_id (us)
    write_string(&mut body, "localhost");
    write_i32(&mut body, 9092);
    Ok(body)
}

// ─── JoinGroup (11) ──────────────────────────────────────────────────────────

fn handle_join_group<S: StreamStorage>(
    req: &KafkaRequest,
    storage: &S,
) -> StreamResult<Vec<u8>> {
    let mut cur = Cursor::new(&req.body);
    let group_id = cur.read_string().unwrap_or_default();
    let session_timeout_ms = cur.read_i32().unwrap_or(30_000);
    let rebalance_timeout_ms = cur.read_i32().unwrap_or(60_000);
    let member_id = cur.read_string().unwrap_or_default();
    let _group_instance_id = cur.read_nullable_string().unwrap_or_default();
    let _protocol_type = cur.read_string().unwrap_or_default();
    let protocol_count = cur.read_i32().unwrap_or(0);

    // Read subscriptions from the first protocol metadata.
    let mut subscriptions: Vec<String> = Vec::new();
    for i in 0..protocol_count {
        let _name = cur.read_string().unwrap_or_default();
        let _meta = cur.read_bytes().unwrap_or_default();
        if i == 0 {
            subscriptions = vec!["*".into()]; // simplified
        }
    }

    let mut group = storage
        .get_or_create_group(&group_id)
        .unwrap_or_else(|_| crate::models::ConsumerGroup::new(&group_id));

    let effective_member_id = if member_id.is_empty() {
        format!("member-{}", uuid::Uuid::new_v4())
    } else {
        member_id.clone()
    };

    let is_leader = group.leader_id.is_none();
    if is_leader {
        group.leader_id = Some(effective_member_id.clone());
    }

    group.members.insert(
        effective_member_id.clone(),
        crate::models::GroupMember {
            member_id: effective_member_id.clone(),
            client_id: req.header.client_id.clone(),
            subscriptions,
            assignments: Vec::new(),
            last_heartbeat_ms: chrono::Utc::now().timestamp_millis(),
            session_timeout_ms,
            rebalance_timeout_ms,
        },
    );

    group.generation += 1;
    group.state = crate::models::GroupState::CompletingRebalance;
    let generation = group.generation;
    let leader_id = group.leader_id.clone().unwrap_or_default();

    let _ = storage.update_group(group);

    let mut body = Vec::new();
    write_i32(&mut body, 0); // throttle_time_ms
    write_i16(&mut body, ERR_NONE);
    write_i32(&mut body, generation);
    write_string(&mut body, "range"); // protocol_name
    write_string(&mut body, &leader_id);
    write_string(&mut body, &effective_member_id);
    // members (only leader gets the full list; simplified: empty)
    write_i32(&mut body, 0);
    Ok(body)
}

// ─── SyncGroup (14) ──────────────────────────────────────────────────────────

fn handle_sync_group<S: StreamStorage>(
    req: &KafkaRequest,
    storage: &S,
) -> StreamResult<Vec<u8>> {
    let mut cur = Cursor::new(&req.body);
    let group_id = cur.read_string().unwrap_or_default();
    let generation = cur.read_i32().unwrap_or(0);
    let member_id = cur.read_string().unwrap_or_default();
    let _group_instance_id = cur.read_nullable_string().unwrap_or_default();
    let _protocol_type = cur.read_nullable_string().unwrap_or_default();
    let _protocol_name = cur.read_nullable_string().unwrap_or_default();

    // Mark group as stable.
    if let Ok(mut group) = storage.get_or_create_group(&group_id) {
        group.state = crate::models::GroupState::Stable;
        let _ = storage.update_group(group);
    }

    let mut body = Vec::new();
    write_i32(&mut body, 0); // throttle_time_ms
    write_i16(&mut body, ERR_NONE);
    write_string(&mut body, "consumer"); // protocol_type
    write_string(&mut body, "range"); // protocol_name
    // assignment bytes (empty = no partitions assigned via wire protocol)
    write_i32(&mut body, 0);
    Ok(body)
}

// ─── Heartbeat (12) ──────────────────────────────────────────────────────────

fn handle_heartbeat<S: StreamStorage>(
    req: &KafkaRequest,
    storage: &S,
) -> StreamResult<Vec<u8>> {
    let mut cur = Cursor::new(&req.body);
    let group_id = cur.read_string().unwrap_or_default();
    let _generation = cur.read_i32().unwrap_or(0);
    let member_id = cur.read_string().unwrap_or_default();

    let err = if let Ok(mut group) = storage.get_or_create_group(&group_id) {
        if let Some(member) = group.members.get_mut(&member_id) {
            member.last_heartbeat_ms = chrono::Utc::now().timestamp_millis();
            let _ = storage.update_group(group);
            ERR_NONE
        } else {
            ERR_UNKNOWN_MEMBER
        }
    } else {
        ERR_UNKNOWN_MEMBER
    };

    let mut body = Vec::new();
    write_i32(&mut body, 0); // throttle_time_ms
    write_i16(&mut body, err);
    Ok(body)
}

// ─── LeaveGroup (13) ─────────────────────────────────────────────────────────

fn handle_leave_group<S: StreamStorage>(
    req: &KafkaRequest,
    storage: &S,
) -> StreamResult<Vec<u8>> {
    let mut cur = Cursor::new(&req.body);
    let group_id = cur.read_string().unwrap_or_default();
    let member_id = cur.read_string().unwrap_or_default();

    if let Ok(mut group) = storage.get_or_create_group(&group_id) {
        group.members.remove(&member_id);
        if group.members.is_empty() {
            group.state = crate::models::GroupState::Empty;
        } else {
            group.state = crate::models::GroupState::PreparingRebalance;
            group.generation += 1;
        }
        let _ = storage.update_group(group);
    }

    let mut body = Vec::new();
    write_i32(&mut body, 0); // throttle_time_ms
    write_i16(&mut body, ERR_NONE);
    // members (leave members list — v4+)
    write_i32(&mut body, 1);
    write_string(&mut body, &member_id);
    write_string(&mut body, ""); // group_instance_id
    write_i16(&mut body, ERR_NONE);
    Ok(body)
}

// ─── Binary encoding helpers ──────────────────────────────────────────────────

fn write_i8(buf: &mut Vec<u8>, v: i8) {
    buf.push(v as u8);
}
fn write_i16(buf: &mut Vec<u8>, v: i16) {
    buf.extend_from_slice(&v.to_be_bytes());
}
fn write_i32(buf: &mut Vec<u8>, v: i32) {
    buf.extend_from_slice(&v.to_be_bytes());
}
fn write_i64(buf: &mut Vec<u8>, v: i64) {
    buf.extend_from_slice(&v.to_be_bytes());
}
fn write_bool(buf: &mut Vec<u8>, v: bool) {
    buf.push(if v { 1 } else { 0 });
}
fn write_string(buf: &mut Vec<u8>, s: &str) {
    let bytes = s.as_bytes();
    write_i16(buf, bytes.len() as i16);
    buf.extend_from_slice(bytes);
}
fn write_bytes(buf: &mut Vec<u8>, data: &[u8]) {
    write_i32(buf, data.len() as i32);
    buf.extend_from_slice(data);
}

// ─── Cursor / reader ─────────────────────────────────────────────────────────

struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn read_i8(&mut self) -> StreamResult<i8> {
        self.need(1)?;
        let v = self.data[self.pos] as i8;
        self.pos += 1;
        Ok(v)
    }

    fn read_i16(&mut self) -> StreamResult<i16> {
        self.need(2)?;
        let v = i16::from_be_bytes([self.data[self.pos], self.data[self.pos + 1]]);
        self.pos += 2;
        Ok(v)
    }

    fn read_i32(&mut self) -> StreamResult<i32> {
        self.need(4)?;
        let b = &self.data[self.pos..self.pos + 4];
        let v = i32::from_be_bytes([b[0], b[1], b[2], b[3]]);
        self.pos += 4;
        Ok(v)
    }

    fn read_i64(&mut self) -> StreamResult<i64> {
        self.need(8)?;
        let b = &self.data[self.pos..self.pos + 8];
        let v = i64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]);
        self.pos += 8;
        Ok(v)
    }

    fn read_string(&mut self) -> StreamResult<String> {
        let len = self.read_i16()? as usize;
        self.need(len)?;
        let s = std::str::from_utf8(&self.data[self.pos..self.pos + len])
            .map_err(|e| StreamError::Protocol(format!("Invalid UTF-8 string: {e}")))?
            .to_string();
        self.pos += len;
        Ok(s)
    }

    fn read_nullable_string(&mut self) -> StreamResult<String> {
        let len = self.read_i16()?;
        if len < 0 {
            return Ok(String::new());
        }
        let len = len as usize;
        self.need(len)?;
        let s = std::str::from_utf8(&self.data[self.pos..self.pos + len])
            .unwrap_or("")
            .to_string();
        self.pos += len;
        Ok(s)
    }

    fn read_bytes(&mut self) -> StreamResult<Vec<u8>> {
        let len = self.read_i32()?;
        if len < 0 {
            return Ok(Vec::new());
        }
        let len = len as usize;
        self.need(len)?;
        let v = self.data[self.pos..self.pos + len].to_vec();
        self.pos += len;
        Ok(v)
    }

    fn remaining(&self) -> &[u8] {
        &self.data[self.pos..]
    }

    fn need(&self, n: usize) -> StreamResult<()> {
        if self.pos + n > self.data.len() {
            Err(StreamError::Protocol(format!(
                "Buffer underflow: need {n} bytes at pos {}, have {}",
                self.pos,
                self.data.len()
            )))
        } else {
            Ok(())
        }
    }
}

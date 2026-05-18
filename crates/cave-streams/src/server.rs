// SPDX-License-Identifier: AGPL-3.0-or-later
//! TCP Kafka wire protocol server.
//!
//! Listens on KAFKA_PORT (9092) and handles the binary Kafka protocol.
//! Uses tokio-util's LengthDelimitedCodec for framing.

use crate::broker::Broker;
use crate::compression::Codec;
use crate::error::{StreamsError, StreamsResult};
use crate::protocol::{
    self, ApiKey, RequestHeader,
    build_api_versions_response, encode_string, encode_array, frame_response,
    CreateTopicsRequest, FetchRequest, MetadataRequest, ProduceRequest, ProduceResponse,
    ProduceTopicResponse, ProducePartitionResponse,
};
use bytes::{Buf, BufMut, Bytes, BytesMut};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tracing::{debug, error, info, warn};

/// Start the Kafka TCP protocol server.
pub async fn run(broker: Arc<Broker>, port: u16) -> std::io::Result<()> {
    let addr = format!("0.0.0.0:{port}");
    let listener = TcpListener::bind(&addr).await?;
    info!(addr, "CAVE Streams Kafka wire protocol server listening");

    loop {
        match listener.accept().await {
            Ok((stream, peer)) => {
                let broker = Arc::clone(&broker);
                debug!(%peer, "new Kafka client connection");
                tokio::spawn(async move {
                    if let Err(e) = handle_connection(stream, broker).await {
                        debug!(%peer, error = %e, "connection closed");
                    }
                });
            }
            Err(e) => {
                error!(error = %e, "accept failed");
            }
        }
    }
}

async fn handle_connection(mut stream: TcpStream, broker: Arc<Broker>) -> std::io::Result<()> {
    loop {
        // Read 4-byte length prefix
        let mut len_buf = [0u8; 4];
        match stream.read_exact(&mut len_buf).await {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e),
        }
        let msg_len = i32::from_be_bytes(len_buf) as usize;
        if msg_len == 0 {
            continue;
        }

        // Read the full message body
        let mut body = vec![0u8; msg_len];
        stream.read_exact(&mut body).await?;
        let mut buf = Bytes::from(body);

        // Dispatch
        let response = match dispatch_request(&mut buf, &broker).await {
            Ok(r) => r,
            Err(e) => {
                warn!(error = %e, "request dispatch error");
                // Return a generic error response (correlation_id = 0 since we might not have parsed it)
                Bytes::new()
            }
        };

        if !response.is_empty() {
            stream.write_all(&response).await?;
        }
    }
    Ok(())
}

async fn dispatch_request(
    buf: &mut Bytes,
    broker: &Arc<Broker>,
) -> StreamsResult<Bytes> {
    let header = RequestHeader::decode(buf)?;
    let correlation_id = header.correlation_id;

    let response_body = match header.api_key {
        ApiKey::ApiVersions => build_api_versions_response().freeze(),

        ApiKey::Metadata => handle_metadata(buf, broker, header.api_version)?,

        ApiKey::Produce => handle_produce(buf, broker, header.api_version)?,

        ApiKey::Fetch => handle_fetch(buf, broker, header.api_version)?,

        ApiKey::CreateTopics => handle_create_topics(buf, broker, header.api_version)?,

        ApiKey::DeleteTopics => handle_delete_topics(buf, broker)?,

        ApiKey::ListOffsets => handle_list_offsets(buf, broker)?,

        ApiKey::OffsetFetch => handle_offset_fetch(buf, broker)?,

        ApiKey::OffsetCommit => handle_offset_commit(buf, broker)?,

        ApiKey::FindCoordinator => handle_find_coordinator(broker)?,

        ApiKey::JoinGroup => handle_join_group(buf, broker)?,

        ApiKey::SyncGroup => handle_sync_group(buf, broker)?,

        ApiKey::Heartbeat => handle_heartbeat(buf, broker)?,

        ApiKey::LeaveGroup => handle_leave_group(buf, broker)?,

        ApiKey::ListGroups => handle_list_groups(broker)?,

        ApiKey::DescribeGroups => handle_describe_groups(buf, broker)?,

        ApiKey::DeleteGroups => handle_delete_groups(buf, broker)?,

        ApiKey::InitProducerId => handle_init_producer_id(buf, broker)?,

        ApiKey::DeleteRecords => handle_delete_records(buf, broker)?,

        ApiKey::Vote => handle_kraft_vote(buf, broker)?,
        ApiKey::BeginQuorumEpoch => handle_kraft_begin_quorum_epoch(buf, broker)?,
        ApiKey::EndQuorumEpoch => handle_kraft_end_quorum_epoch(buf, broker)?,
        ApiKey::DescribeQuorum => handle_kraft_describe_quorum(buf, broker)?,

        _ => {
            // Unsupported API — return empty error response
            let mut b = BytesMut::new();
            b.put_i16(35); // UNSUPPORTED_VERSION
            b.freeze()
        }
    };

    Ok(frame_response(correlation_id, response_body))
}

// ── Handler helpers ───────────────────────────────────────────────────────────

fn handle_metadata(buf: &mut Bytes, broker: &Broker, version: i16) -> StreamsResult<Bytes> {
    let req = MetadataRequest::decode(buf, version)?;
    let topics = req.topics.as_deref().unwrap_or(&[]);
    let mut b = BytesMut::new();
    b.put_i32(0); // throttle_time_ms
    // brokers array: [id, host, port, rack]
    b.put_i32(1);
    b.put_i32(broker.broker_id());
    encode_string(&mut b, &broker.config.host);
    b.put_i32(broker.config.port as i32);
    b.put_i16(-1); // rack = null
    // cluster_id
    encode_string(&mut b, broker.cluster_id());
    b.put_i32(broker.controller_id());
    // topics array
    let topic_list: Vec<String> = if topics.is_empty() {
        broker.list_topics()
    } else {
        topics.to_vec()
    };
    b.put_i32(topic_list.len() as i32);
    for topic in &topic_list {
        let num_partitions = broker.topic_partition_count(topic).unwrap_or(0);
        b.put_i16(0); // error_code
        encode_string(&mut b, topic);
        b.put_u8(0); // is_internal = false
        b.put_i32(num_partitions);
        for p in 0..num_partitions {
            b.put_i16(0); // error_code
            b.put_i32(p);
            b.put_i32(broker.broker_id()); // leader
            b.put_i32(0); // leader_epoch
            b.put_i32(1); // replicas array length
            b.put_i32(broker.broker_id());
            b.put_i32(1); // isr array length
            b.put_i32(broker.broker_id());
            b.put_i32(0); // offline_replicas array length
        }
    }
    Ok(b.freeze())
}

fn handle_produce(buf: &mut Bytes, broker: &Broker, version: i16) -> StreamsResult<Bytes> {
    let req = ProduceRequest::decode(buf, version)?;
    let mut responses: Vec<ProduceTopicResponse> = Vec::new();

    for topic_data in req.topic_data {
        let mut partition_responses = Vec::new();
        for pd in topic_data.partition_data {
            let result = broker.produce(
                &topic_data.name,
                pd.index,
                pd.records,
                -1, // producer_id (simplified)
                0,
                0,
                false,
                Codec::None,
            );
            let (error_code, base_offset) = match result {
                Ok(offset) => (0i16, offset),
                Err(e) => (e.kafka_error_code(), -1),
            };
            partition_responses.push(ProducePartitionResponse {
                index: pd.index,
                error_code,
                base_offset,
                log_append_time_ms: chrono::Utc::now().timestamp_millis(),
                log_start_offset: 0,
            });
        }
        responses.push(ProduceTopicResponse {
            name: topic_data.name,
            partition_responses,
        });
    }

    let resp = ProduceResponse { responses, throttle_time_ms: 0 };
    let mut b = BytesMut::new();
    resp.encode(&mut b);
    Ok(b.freeze())
}

fn handle_fetch(buf: &mut Bytes, broker: &Broker, version: i16) -> StreamsResult<Bytes> {
    let req = FetchRequest::decode(buf, version)?;
    let mut b = BytesMut::new();
    b.put_i32(0); // throttle_time_ms
    b.put_i16(0); // error_code
    b.put_i32(0); // session_id
    // responses array
    b.put_i32(req.topics.len() as i32);
    for topic in req.topics {
        encode_string(&mut b, &topic.name);
        b.put_i32(topic.partitions.len() as i32);
        for part in topic.partitions {
            b.put_i32(part.partition);
            let (error_code, hw, lso, records_bytes) = match broker.fetch(
                &topic.name,
                part.partition,
                part.fetch_offset,
                part.partition_max_bytes,
            ) {
                Ok(batches) => {
                    let hw = broker.log_end_offset(&topic.name, part.partition).unwrap_or(0);
                    let mut data = BytesMut::new();
                    for batch in batches {
                        data.put_slice(&batch.data);
                    }
                    (0i16, hw, 0i64, data.freeze())
                }
                Err(_) => (3i16, 0i64, 0i64, Bytes::new()),
            };
            b.put_i16(error_code);
            b.put_i64(hw); // high_watermark
            b.put_i64(lso); // last_stable_offset
            b.put_i64(0); // log_start_offset
            b.put_i32(0); // aborted_transactions count
            b.put_i32(broker.broker_id()); // preferred_read_replica
            b.put_i32(records_bytes.len() as i32);
            b.put_slice(&records_bytes);
        }
    }
    Ok(b.freeze())
}

fn handle_create_topics(buf: &mut Bytes, broker: &Broker, version: i16) -> StreamsResult<Bytes> {
    let req = CreateTopicsRequest::decode(buf, version)?;
    let mut b = BytesMut::new();
    b.put_i32(0); // throttle_time_ms
    b.put_i32(req.topics.len() as i32);
    for topic in req.topics {
        encode_string(&mut b, &topic.name);
        let error_code = match broker.create_topic(
            topic.name.clone(),
            topic.num_partitions,
            topic.replication_factor,
            topic.configs,
        ) {
            Ok(()) => 0i16,
            Err(e) => e.kafka_error_code(),
        };
        b.put_i16(error_code);
        b.put_i16(-1); // error_message = null
    }
    Ok(b.freeze())
}

fn handle_delete_topics(buf: &mut Bytes, broker: &Broker) -> StreamsResult<Bytes> {
    let topics = protocol::decode_array(buf, |b| protocol::decode_string(b))?;
    let _timeout_ms = buf.get_i32();
    let mut b = BytesMut::new();
    b.put_i32(0); // throttle_time_ms
    b.put_i32(topics.len() as i32);
    for topic in topics {
        encode_string(&mut b, &topic);
        let error_code = match broker.delete_topic(&topic) {
            Ok(()) => 0i16,
            Err(e) => e.kafka_error_code(),
        };
        b.put_i16(error_code);
    }
    Ok(b.freeze())
}

fn handle_list_offsets(buf: &mut Bytes, broker: &Broker) -> StreamsResult<Bytes> {
    let _replica_id = buf.get_i32();
    let _isolation_level = buf.get_i8();
    let topics = protocol::decode_array(buf, |b| {
        let name = protocol::decode_string(b)?;
        let partitions = protocol::decode_array(b, |pb| {
            let index = pb.get_i32();
            let _leader_epoch = pb.get_i32();
            let timestamp = pb.get_i64();
            Ok((index, timestamp))
        })?;
        Ok((name, partitions))
    })?;
    let mut b = BytesMut::new();
    b.put_i32(0); // throttle_time_ms
    b.put_i32(topics.len() as i32);
    for (name, partitions) in topics {
        encode_string(&mut b, &name);
        b.put_i32(partitions.len() as i32);
        for (partition, _timestamp) in partitions {
            b.put_i32(partition);
            b.put_i16(0); // error_code
            b.put_i64(broker.log_end_offset(&name, partition).unwrap_or(0));
        }
    }
    Ok(b.freeze())
}

fn handle_offset_fetch(buf: &mut Bytes, broker: &Broker) -> StreamsResult<Bytes> {
    let group = protocol::decode_string(buf)?;
    let topics = protocol::decode_array(buf, |b| {
        let name = protocol::decode_string(b)?;
        let partitions = protocol::decode_array(b, |pb| Ok(pb.get_i32()))?;
        Ok((name, partitions))
    })?;
    let mut b = BytesMut::new();
    b.put_i32(0); // throttle_time_ms
    b.put_i32(topics.len() as i32);
    for (name, partitions) in topics {
        encode_string(&mut b, &name);
        b.put_i32(partitions.len() as i32);
        for partition in partitions {
            b.put_i32(partition);
            let offset = broker.fetch_offset(&group, &name, partition);
            b.put_i64(offset);
            b.put_i16(-1); // metadata = null
            b.put_i16(0); // error_code
        }
    }
    b.put_i16(0); // error_code
    Ok(b.freeze())
}

fn handle_offset_commit(buf: &mut Bytes, broker: &Broker) -> StreamsResult<Bytes> {
    let group = protocol::decode_string(buf)?;
    let _generation_id = buf.get_i32();
    let _member_id = protocol::decode_string(buf)?;
    let topics = protocol::decode_array(buf, |b| {
        let name = protocol::decode_string(b)?;
        let partitions = protocol::decode_array(b, |pb| {
            let index = pb.get_i32();
            let offset = pb.get_i64();
            let _committed_leader_epoch = pb.get_i32();
            let _metadata = protocol::decode_nullable_string(pb)?;
            Ok((index, offset))
        })?;
        Ok((name, partitions))
    })?;
    let mut b = BytesMut::new();
    b.put_i32(0); // throttle_time_ms
    b.put_i32(topics.len() as i32);
    for (name, partitions) in topics {
        encode_string(&mut b, &name);
        b.put_i32(partitions.len() as i32);
        for (partition, offset) in partitions {
            broker.commit_offset(&group, &name, partition, offset);
            b.put_i32(partition);
            b.put_i16(0); // error_code
        }
    }
    Ok(b.freeze())
}

fn handle_find_coordinator(broker: &Broker) -> StreamsResult<Bytes> {
    let mut b = BytesMut::new();
    b.put_i32(0); // throttle_time_ms
    b.put_i16(0); // error_code
    b.put_i16(-1); // error_message = null
    b.put_i32(broker.broker_id()); // node_id
    encode_string(&mut b, &broker.config.host);
    b.put_i32(broker.config.port as i32);
    Ok(b.freeze())
}

fn handle_join_group(buf: &mut Bytes, broker: &Broker) -> StreamsResult<Bytes> {
    let group_id = protocol::decode_string(buf)?;
    let session_timeout = buf.get_i32();
    let rebalance_timeout = buf.get_i32();
    let member_id = protocol::decode_string(buf)?;
    let _group_instance_id = protocol::decode_nullable_string(buf)?;
    let protocol_type = protocol::decode_string(buf)?;
    let protocols = protocol::decode_array(buf, |b| {
        let name = protocol::decode_string(b)?;
        let meta_len = b.get_i32();
        let meta = if meta_len > 0 { b.copy_to_bytes(meta_len as usize).to_vec() } else { vec![] };
        Ok((name, meta))
    })?;

    let protocols_map: std::collections::HashMap<String, Vec<u8>> = protocols.into_iter().collect();
    let result = broker.groups.join_group(
        group_id, Some(member_id), "client".into(), "/127.0.0.1".into(),
        session_timeout, rebalance_timeout, protocol_type, protocols_map,
    )?;

    let mut b = BytesMut::new();
    b.put_i32(0); // throttle_time_ms
    b.put_i16(result.error_code);
    b.put_i32(result.generation_id);
    encode_string(&mut b, &result.protocol_name);
    encode_string(&mut b, &result.leader_id);
    encode_string(&mut b, &result.member_id);
    b.put_i32(result.members.len() as i32);
    for m in result.members {
        encode_string(&mut b, &m.member_id);
        b.put_i16(-1); // group_instance_id = null
        b.put_i32(m.metadata.len() as i32);
        b.put_slice(&m.metadata);
    }
    Ok(b.freeze())
}

fn handle_sync_group(buf: &mut Bytes, broker: &Broker) -> StreamsResult<Bytes> {
    let group_id = protocol::decode_string(buf)?;
    let generation_id = buf.get_i32();
    let member_id = protocol::decode_string(buf)?;
    let _group_instance_id = protocol::decode_nullable_string(buf)?;
    let _protocol_type = protocol::decode_nullable_string(buf)?;
    let _protocol_name = protocol::decode_nullable_string(buf)?;
    let assignments = protocol::decode_array(buf, |b| {
        let mid = protocol::decode_string(b)?;
        let assign_len = b.get_i32();
        let assign = if assign_len > 0 { b.copy_to_bytes(assign_len as usize).to_vec() } else { vec![] };
        Ok((mid, assign))
    })?;
    let assign_map: std::collections::HashMap<String, Vec<u8>> = assignments.into_iter().collect();
    let assignment = broker.groups.sync_group(&group_id, generation_id, &member_id, assign_map)?;
    let mut b = BytesMut::new();
    b.put_i32(0);
    b.put_i16(0);
    b.put_i32(assignment.len() as i32);
    b.put_slice(&assignment);
    Ok(b.freeze())
}

fn handle_heartbeat(buf: &mut Bytes, broker: &Broker) -> StreamsResult<Bytes> {
    let group_id = protocol::decode_string(buf)?;
    let generation_id = buf.get_i32();
    let member_id = protocol::decode_string(buf)?;
    let error_code = broker.groups.heartbeat(&group_id, generation_id, &member_id)?;
    let mut b = BytesMut::new();
    b.put_i32(0);
    b.put_i16(error_code);
    Ok(b.freeze())
}

fn handle_leave_group(buf: &mut Bytes, broker: &Broker) -> StreamsResult<Bytes> {
    let group_id = protocol::decode_string(buf)?;
    let _member_id = protocol::decode_string(buf)?;
    let members = protocol::decode_array(buf, |b| {
        let mid = protocol::decode_string(b)?;
        let _gid = protocol::decode_nullable_string(b)?;
        Ok(mid)
    });
    let member_ids = members.unwrap_or_default();
    for mid in member_ids {
        let _ = broker.groups.leave_group(&group_id, &mid);
    }
    let mut b = BytesMut::new();
    b.put_i32(0);
    b.put_i16(0);
    b.put_i32(0); // members array empty
    Ok(b.freeze())
}

fn handle_list_groups(broker: &Broker) -> StreamsResult<Bytes> {
    let groups = broker.groups.list_groups();
    let mut b = BytesMut::new();
    b.put_i32(0);
    b.put_i16(0);
    b.put_i32(groups.len() as i32);
    for g in groups {
        encode_string(&mut b, &g.group_id);
        encode_string(&mut b, &g.protocol_type);
        encode_string(&mut b, &g.state);
    }
    Ok(b.freeze())
}

fn handle_describe_groups(buf: &mut Bytes, broker: &Broker) -> StreamsResult<Bytes> {
    let group_ids = protocol::decode_array(buf, |b| protocol::decode_string(b))?;
    let mut b = BytesMut::new();
    b.put_i32(0);
    b.put_i32(group_ids.len() as i32);
    for gid in group_ids {
        match broker.groups.describe_group(&gid) {
            Some(desc) => {
                b.put_i16(0);
                encode_string(&mut b, &gid);
                encode_string(&mut b, &desc.state);
                encode_string(&mut b, &desc.protocol_type);
                encode_string(&mut b, &desc.protocol);
                b.put_i32(desc.members.len() as i32);
                for m in desc.members {
                    encode_string(&mut b, &m.member_id);
                    b.put_i16(-1);
                    encode_string(&mut b, &m.client_id);
                    encode_string(&mut b, &m.client_host);
                    b.put_i32(0); // metadata
                    b.put_i32(0); // assignment
                }
            }
            None => {
                b.put_i16(16); // GROUP_ID_NOT_FOUND
                encode_string(&mut b, &gid);
                encode_string(&mut b, "Dead");
                b.put_i16(-1);
                b.put_i16(-1);
                b.put_i32(0);
            }
        }
    }
    Ok(b.freeze())
}

fn handle_delete_groups(buf: &mut Bytes, broker: &Broker) -> StreamsResult<Bytes> {
    let group_ids = protocol::decode_array(buf, |b| protocol::decode_string(b))?;
    let mut b = BytesMut::new();
    b.put_i32(0);
    b.put_i32(group_ids.len() as i32);
    for gid in group_ids {
        let error_code = match broker.groups.delete_group(&gid) {
            Ok(()) => 0i16,
            Err(_) => 16i16,
        };
        encode_string(&mut b, &gid);
        b.put_i16(error_code);
    }
    Ok(b.freeze())
}

fn handle_init_producer_id(buf: &mut Bytes, broker: &Broker) -> StreamsResult<Bytes> {
    let transactional_id = protocol::decode_nullable_string(buf)?;
    let txn_timeout_ms = buf.get_i32();
    let _producer_id = buf.get_i64();
    let _producer_epoch = buf.get_i16();

    let (pid, epoch) = broker.transactions.init_producer(
        transactional_id,
        txn_timeout_ms,
        || broker.allocate_producer_id(),
    )?;
    let mut b = BytesMut::new();
    b.put_i32(0);
    b.put_i16(0);
    b.put_i64(pid);
    b.put_i16(epoch);
    Ok(b.freeze())
}

fn handle_delete_records(buf: &mut Bytes, broker: &Broker) -> StreamsResult<Bytes> {
    let topics = protocol::decode_array(buf, |b| {
        let name = protocol::decode_string(b)?;
        let partitions = protocol::decode_array(b, |pb| {
            let index = pb.get_i32();
            let before_offset = pb.get_i64();
            Ok((index, before_offset))
        })?;
        Ok((name, partitions))
    })?;
    let _timeout_ms = buf.get_i32();

    let mut b = BytesMut::new();
    b.put_i32(0);
    b.put_i32(topics.len() as i32);
    for (name, partitions) in topics {
        encode_string(&mut b, &name);
        b.put_i32(partitions.len() as i32);
        for (partition, before_offset) in partitions {
            b.put_i32(partition);
            let (error_code, low_watermark) = match broker.delete_records(&name, partition, before_offset) {
                Ok(lso) => (0i16, lso),
                Err(e) => (e.kafka_error_code(), -1),
            };
            b.put_i64(low_watermark);
            b.put_i16(error_code);
        }
    }
    Ok(b.freeze())
}

// ── KRaft RPCs (KIP-595) ──────────────────────────────────────────────────────
//
// These four handlers route incoming Vote / BeginQuorumEpoch /
// EndQuorumEpoch / DescribeQuorum requests to the
// `KraftHandler` installed on the broker via
// `Broker::set_kraft_handler`. If no handler is installed (the
// broker isn't part of a controller quorum), we return error
// code 50 (REASSIGNMENT_IN_PROGRESS — Kafka's nearest
// "this broker can't currently serve this request" code).

const ERR_BROKER_NOT_AVAILABLE: i16 = 8;

fn handle_kraft_vote(buf: &mut Bytes, broker: &Broker) -> StreamsResult<Bytes> {
    use crate::kraft::rpc::{VoteRequest, VoteResponse};
    let req = VoteRequest::decode(buf)?;
    let mut b = BytesMut::new();
    match broker.kraft_handler() {
        Some(h) => h.handle_vote(&req).encode(&mut b),
        None => {
            let resp = VoteResponse {
                error_code: ERR_BROKER_NOT_AVAILABLE,
                topic_name: req.topic_name,
                partition_index: req.partition_index,
                leader_id: -1,
                leader_epoch: -1,
                vote_granted: false,
            };
            resp.encode(&mut b);
        }
    }
    Ok(b.freeze())
}

fn handle_kraft_begin_quorum_epoch(buf: &mut Bytes, broker: &Broker) -> StreamsResult<Bytes> {
    use crate::kraft::rpc::{BeginQuorumEpochRequest, BeginQuorumEpochResponse};
    let req = BeginQuorumEpochRequest::decode(buf)?;
    let mut b = BytesMut::new();
    match broker.kraft_handler() {
        Some(h) => h.handle_begin_quorum_epoch(&req).encode(&mut b),
        None => BeginQuorumEpochResponse {
            error_code: ERR_BROKER_NOT_AVAILABLE,
            topic_name: req.topic_name,
            partition_index: req.partition_index,
        }
        .encode(&mut b),
    }
    Ok(b.freeze())
}

fn handle_kraft_end_quorum_epoch(buf: &mut Bytes, broker: &Broker) -> StreamsResult<Bytes> {
    use crate::kraft::rpc::{EndQuorumEpochRequest, EndQuorumEpochResponse};
    let req = EndQuorumEpochRequest::decode(buf)?;
    let mut b = BytesMut::new();
    match broker.kraft_handler() {
        Some(h) => h.handle_end_quorum_epoch(&req).encode(&mut b),
        None => EndQuorumEpochResponse {
            error_code: ERR_BROKER_NOT_AVAILABLE,
            topic_name: req.topic_name,
            partition_index: req.partition_index,
        }
        .encode(&mut b),
    }
    Ok(b.freeze())
}

fn handle_kraft_describe_quorum(buf: &mut Bytes, broker: &Broker) -> StreamsResult<Bytes> {
    use crate::kraft::rpc::{DescribeQuorumRequest, DescribeQuorumResponse};
    let req = DescribeQuorumRequest::decode(buf)?;
    let mut b = BytesMut::new();
    match broker.kraft_handler() {
        Some(h) => h.handle_describe_quorum(&req).encode(&mut b),
        None => DescribeQuorumResponse {
            error_code: ERR_BROKER_NOT_AVAILABLE,
            topic_name: req.topic_name,
            partition_index: req.partition_index,
            leader_id: -1,
            leader_epoch: -1,
            high_watermark: -1,
            current_voters: Vec::new(),
            observers: Vec::new(),
        }
        .encode(&mut b),
    }
    Ok(b.freeze())
}

#[cfg(test)]
mod kraft_dispatch_tests {
    use super::*;
    use crate::broker::BrokerConfig;
    use crate::kraft::{KraftHandler, MetadataLog, VoterSet};
    use crate::protocol::encode_string;
    use std::sync::Arc;

    fn broker_with_kraft() -> Arc<Broker> {
        let cfg = BrokerConfig::default();
        let broker = Arc::new(Broker::new(cfg));
        let handler = KraftHandler::new(VoterSet::new([1, 2, 3]), Arc::new(MetadataLog::new()));
        broker.set_kraft_handler(Arc::new(handler));
        broker
    }

    fn encode_request_with_header(api_key: ApiKey, body: &[u8]) -> Bytes {
        let mut b = BytesMut::new();
        b.put_i16(api_key as i16);
        b.put_i16(0); // api_version
        b.put_i32(42); // correlation_id
        encode_string(&mut b, "test-client");
        b.put_slice(body);
        b.freeze()
    }

    #[tokio::test]
    async fn dispatch_vote_returns_response_payload() {
        use crate::kraft::rpc::{VoteRequest, VoteResponse};
        let broker = broker_with_kraft();
        let req = VoteRequest {
            topic_name: "__cluster_metadata".into(),
            partition_index: 0,
            candidate_epoch: 1,
            candidate_id: 2,
            last_offset_epoch: 0,
            last_offset: 0,
        };
        let mut body = BytesMut::new();
        req.encode(&mut body);

        let mut wire = encode_request_with_header(ApiKey::Vote, &body);
        let resp = dispatch_request(&mut wire, &broker).await.unwrap();
        // Strip the frame header (4-byte len + 4-byte correlation).
        let mut payload = &resp[8..];
        let r = VoteResponse::decode(&mut payload).unwrap();
        assert!(r.vote_granted);
    }

    #[tokio::test]
    async fn dispatch_describe_quorum_returns_payload() {
        use crate::kraft::rpc::{DescribeQuorumRequest, DescribeQuorumResponse};
        let broker = broker_with_kraft();
        let req = DescribeQuorumRequest {
            topic_name: "__cluster_metadata".into(),
            partition_index: 0,
        };
        let mut body = BytesMut::new();
        req.encode(&mut body);
        let mut wire = encode_request_with_header(ApiKey::DescribeQuorum, &body);
        let resp = dispatch_request(&mut wire, &broker).await.unwrap();
        let mut payload = &resp[8..];
        let r = DescribeQuorumResponse::decode(&mut payload).unwrap();
        assert_eq!(r.error_code, 0);
    }

    #[tokio::test]
    async fn dispatch_kraft_without_handler_yields_not_available() {
        use crate::kraft::rpc::{DescribeQuorumRequest, DescribeQuorumResponse};
        // Broker without set_kraft_handler.
        let broker = Arc::new(Broker::new(BrokerConfig::default()));
        let req = DescribeQuorumRequest {
            topic_name: "__cluster_metadata".into(),
            partition_index: 0,
        };
        let mut body = BytesMut::new();
        req.encode(&mut body);
        let mut wire = encode_request_with_header(ApiKey::DescribeQuorum, &body);
        let resp = dispatch_request(&mut wire, &broker).await.unwrap();
        let mut payload = &resp[8..];
        let r = DescribeQuorumResponse::decode(&mut payload).unwrap();
        assert_eq!(r.error_code, ERR_BROKER_NOT_AVAILABLE);
    }
}

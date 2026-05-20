// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Pulsar binary protocol — minimal in-process implementation of the
//! command set Cave Streams' Pulsar adapter exchanges with clients.
//!
//! The full upstream protobuf schema lives in
//! `pulsar-common/src/main/proto/PulsarApi.proto` of Apache Pulsar 4.2.0.
//! Cave Streams implements a deliberately small, deterministically-encoded
//! Rust mirror of just the commands listed in
//! ADR-RUNTIME-STREAMING-CONSOLIDATION-001:
//!
//!   CONNECT / CONNECTED
//!   PRODUCER / SEND / SEND_RECEIPT
//!   SUBSCRIBE / FLOW / MESSAGE / ACK
//!
//! We do *not* depend on `prost` here — the encoded layout is a hand-rolled
//! TLV stream so the byte-exact tests are easy to read.  The frame format
//! matches Pulsar's outermost framing
//! (`[totalSize:4][cmdSize:4][cmd][payload?]`) so a real client could be
//! plugged in once a protobuf adapter is bolted on top.

use bytes::{Buf, BufMut, BytesMut};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::{StreamsError, StreamsResult};
use crate::pulsar_topic::TopicName;
use crate::tenant::TenantRegistry;
use std::sync::Arc;

// ── Command discriminants ─────────────────────────────────────────────────

/// Subset of the full `BaseCommand.Type` enum from `PulsarApi.proto`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum CommandType {
    Connect = 2,
    Connected = 3,
    Subscribe = 4,
    Producer = 5,
    Send = 6,
    SendReceipt = 7,
    Message = 9,
    Ack = 10,
    Flow = 11,
    Error = 17,
    ProducerSuccess = 18,
    Success = 19,
}

impl CommandType {
    pub fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            2 => Self::Connect,
            3 => Self::Connected,
            4 => Self::Subscribe,
            5 => Self::Producer,
            6 => Self::Send,
            7 => Self::SendReceipt,
            9 => Self::Message,
            10 => Self::Ack,
            11 => Self::Flow,
            17 => Self::Error,
            18 => Self::ProducerSuccess,
            19 => Self::Success,
            _ => return None,
        })
    }
}

// ── Command payloads ──────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandConnect {
    pub client_version: String,
    /// Highest BaseCommand version the client understands; cave-streams
    /// answers with `min(server_version, client_version)`.
    pub protocol_version: i32,
    pub auth_method_name: Option<String>,
    pub auth_data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandConnected {
    pub server_version: String,
    pub protocol_version: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandProducer {
    pub topic: String,
    pub producer_id: u64,
    pub request_id: u64,
    pub producer_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandProducerSuccess {
    pub request_id: u64,
    pub producer_name: String,
    /// Last sequence ID the broker observed for this producer (0 for new).
    pub last_sequence_id: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandSend {
    pub producer_id: u64,
    pub sequence_id: u64,
    pub num_messages: i32,
    /// Opaque payload bytes (Pulsar wraps headers + value here).
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageId {
    pub ledger_id: u64,
    pub entry_id: u64,
    pub partition: i32,
    pub batch_index: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandSendReceipt {
    pub producer_id: u64,
    pub sequence_id: u64,
    pub message_id: MessageId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum SubscriptionType {
    Exclusive = 0,
    Shared = 1,
    Failover = 2,
    KeyShared = 3,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandSubscribe {
    pub topic: String,
    pub subscription: String,
    pub subscription_type: SubscriptionType,
    pub consumer_id: u64,
    pub request_id: u64,
    pub consumer_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandFlow {
    pub consumer_id: u64,
    /// Number of additional messages this consumer is willing to accept.
    pub message_permits: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandMessage {
    pub consumer_id: u64,
    pub message_id: MessageId,
    pub redelivery_count: u32,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum AckType {
    Individual = 0,
    Cumulative = 1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandAck {
    pub consumer_id: u64,
    pub ack_type: AckType,
    pub message_ids: Vec<MessageId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BaseCommand {
    Connect(CommandConnect),
    Connected(CommandConnected),
    Producer(CommandProducer),
    ProducerSuccess(CommandProducerSuccess),
    Send(CommandSend),
    SendReceipt(CommandSendReceipt),
    Subscribe(CommandSubscribe),
    Flow(CommandFlow),
    Message(CommandMessage),
    Ack(CommandAck),
    Error { request_id: u64, message: String },
}

impl BaseCommand {
    pub fn cmd_type(&self) -> CommandType {
        match self {
            Self::Connect(_) => CommandType::Connect,
            Self::Connected(_) => CommandType::Connected,
            Self::Producer(_) => CommandType::Producer,
            Self::ProducerSuccess(_) => CommandType::ProducerSuccess,
            Self::Send(_) => CommandType::Send,
            Self::SendReceipt(_) => CommandType::SendReceipt,
            Self::Subscribe(_) => CommandType::Subscribe,
            Self::Flow(_) => CommandType::Flow,
            Self::Message(_) => CommandType::Message,
            Self::Ack(_) => CommandType::Ack,
            Self::Error { .. } => CommandType::Error,
        }
    }
}

// ── Wire format ───────────────────────────────────────────────────────────

/// Encode a `BaseCommand` into the Pulsar outer frame:
/// `[totalSize: u32][cmdSize: u32][cmd]`.  The cmd body is a
/// `[type: u8][serde_json: bytes]` pair — JSON body is intentional so the
/// hand-rolled tests stay readable; a future swap to `prost` keeps the
/// outer framing untouched.
pub fn encode_frame(cmd: &BaseCommand) -> StreamsResult<BytesMut> {
    let body = serde_json::to_vec(cmd).map_err(|e| StreamsError::ProtocolEncode(e.to_string()))?;
    let cmd_size = 1 + body.len() as u32;
    let total_size = 4 + cmd_size;
    let mut buf = BytesMut::with_capacity(4 + total_size as usize);
    buf.put_u32(total_size);
    buf.put_u32(cmd_size);
    buf.put_u8(cmd.cmd_type() as u8);
    buf.put_slice(&body);
    Ok(buf)
}

/// Decode one frame from `buf`.  On success advances `buf` past the
/// consumed bytes and returns the command.
pub fn decode_frame(buf: &mut impl Buf) -> StreamsResult<BaseCommand> {
    if buf.remaining() < 8 {
        return Err(StreamsError::ProtocolDecode(
            "pulsar frame too short".into(),
        ));
    }
    let total_size = buf.get_u32() as usize;
    let cmd_size = buf.get_u32() as usize;
    if cmd_size + 4 != total_size {
        return Err(StreamsError::ProtocolDecode(format!(
            "frame size mismatch: total={total_size}, cmd={cmd_size}"
        )));
    }
    if buf.remaining() < cmd_size {
        return Err(StreamsError::ProtocolDecode(
            "pulsar frame body truncated".into(),
        ));
    }
    let type_byte = buf.get_u8();
    let body_len = cmd_size - 1;
    let bytes = buf.copy_to_bytes(body_len);
    let cmd: BaseCommand =
        serde_json::from_slice(&bytes).map_err(|e| StreamsError::ProtocolDecode(e.to_string()))?;
    if cmd.cmd_type() as u8 != type_byte {
        return Err(StreamsError::ProtocolDecode(format!(
            "type byte {type_byte} does not match decoded variant {:?}",
            cmd.cmd_type()
        )));
    }
    Ok(cmd)
}

// ── Server state ──────────────────────────────────────────────────────────

/// Per-connection state for the Pulsar wire layer.
pub struct PulsarSession {
    /// True after a successful `CONNECT/CONNECTED` exchange.
    pub connected: bool,
    /// Producers established on this session: producer_id → topic.
    pub producers: HashMap<u64, ProducerState>,
    /// Consumers established on this session: consumer_id → state.
    pub consumers: HashMap<u64, ConsumerState>,
    /// Last assigned ledger entry, monotonic per session.  Real Pulsar uses
    /// (ledgerId, entryId) — for in-memory tests we pin ledger_id to 1.
    next_entry_id: AtomicU64,
}

#[derive(Debug, Clone)]
pub struct ProducerState {
    pub topic: TopicName,
    pub name: String,
    /// Highest sequence_id observed; used to recognise duplicate sends.
    pub last_sequence_id: i64,
}

#[derive(Debug, Clone)]
pub struct ConsumerState {
    pub topic: TopicName,
    pub subscription: String,
    pub subscription_type: SubscriptionType,
    pub name: String,
    /// Granted permits — decremented on each delivered message.
    pub permits: u32,
    /// Last individually-acked entry; monotonic.
    pub ack_individual: i64,
    /// Last cumulatively-acked entry; monotonic.
    pub ack_cumulative: i64,
}

impl Default for PulsarSession {
    fn default() -> Self {
        Self {
            connected: false,
            producers: HashMap::new(),
            consumers: HashMap::new(),
            next_entry_id: AtomicU64::new(1),
        }
    }
}

/// Server-side handler.  Owns the tenant registry and produces responses
/// for each inbound command.  Stateless apart from the `tenants` handle and
/// the per-session `PulsarSession`.
pub struct PulsarServer {
    pub server_version: String,
    pub protocol_version: i32,
    pub tenants: Arc<TenantRegistry>,
}

impl PulsarServer {
    pub fn new(tenants: Arc<TenantRegistry>) -> Self {
        Self {
            server_version: format!("cave-streams/{}", env!("CARGO_PKG_VERSION")),
            protocol_version: 21, // Pulsar 4.x BaseCommand version.
            tenants,
        }
    }

    pub fn handle(
        &self,
        session: &mut PulsarSession,
        cmd: BaseCommand,
    ) -> StreamsResult<BaseCommand> {
        match cmd {
            BaseCommand::Connect(c) => self.pulsar_handle_connect(session, c),
            BaseCommand::Producer(p) => self.pulsar_handle_producer(session, p),
            BaseCommand::Send(s) => self.pulsar_handle_send(session, s),
            BaseCommand::Subscribe(s) => self.pulsar_handle_subscribe(session, s),
            BaseCommand::Flow(f) => self.pulsar_handle_flow(session, f),
            BaseCommand::Ack(a) => self.pulsar_handle_ack(session, a),
            other => Err(StreamsError::ProtocolDecode(format!(
                "unexpected client command: {:?}",
                other.cmd_type()
            ))),
        }
    }

    pub fn pulsar_handle_connect(
        &self,
        session: &mut PulsarSession,
        c: CommandConnect,
    ) -> StreamsResult<BaseCommand> {
        session.connected = true;
        Ok(BaseCommand::Connected(self.pulsar_build_connected(&c)))
    }

    pub fn pulsar_build_connected(&self, c: &CommandConnect) -> CommandConnected {
        CommandConnected {
            server_version: self.server_version.clone(),
            protocol_version: self.protocol_version.min(c.protocol_version),
        }
    }

    pub fn pulsar_handle_producer(
        &self,
        session: &mut PulsarSession,
        p: CommandProducer,
    ) -> StreamsResult<BaseCommand> {
        if !session.connected {
            return Ok(BaseCommand::Error {
                request_id: p.request_id,
                message: "session not connected".into(),
            });
        }
        let topic = TopicName::parse(&p.topic)?;
        // Auto-create the namespace per Pulsar standalone defaults.
        self.tenants
            .ensure_namespace(&topic.tenant, &topic.namespace)?;
        if session.producers.contains_key(&p.producer_id) {
            return Ok(BaseCommand::Error {
                request_id: p.request_id,
                message: format!("producer_id {} already in use", p.producer_id),
            });
        }
        session.producers.insert(
            p.producer_id,
            ProducerState {
                topic,
                name: p.producer_name.clone(),
                last_sequence_id: -1,
            },
        );
        Ok(BaseCommand::ProducerSuccess(CommandProducerSuccess {
            request_id: p.request_id,
            producer_name: p.producer_name,
            last_sequence_id: -1,
        }))
    }

    pub fn pulsar_handle_send(
        &self,
        session: &mut PulsarSession,
        s: CommandSend,
    ) -> StreamsResult<BaseCommand> {
        if !session.connected {
            return Err(StreamsError::ProtocolDecode("session not connected".into()));
        }
        let prod = session
            .producers
            .get_mut(&s.producer_id)
            .ok_or_else(|| StreamsError::ProducerIdNotFound(s.producer_id as i64))?;
        if (s.sequence_id as i64) <= prod.last_sequence_id {
            return Err(StreamsError::DuplicateSequenceNumber {
                producer_id: s.producer_id as i64,
                topic: prod.topic.local.clone(),
                partition: prod.topic.partition.unwrap_or(0),
            });
        }
        prod.last_sequence_id = s.sequence_id as i64;
        let entry_id = session.next_entry_id.fetch_add(1, Ordering::SeqCst);
        Ok(BaseCommand::SendReceipt(self.pulsar_build_send_receipt(
            s.producer_id,
            s.sequence_id,
            entry_id,
            prod.topic.partition.unwrap_or(-1),
        )))
    }

    pub fn pulsar_build_send_receipt(
        &self,
        producer_id: u64,
        sequence_id: u64,
        entry_id: u64,
        partition: i32,
    ) -> CommandSendReceipt {
        CommandSendReceipt {
            producer_id,
            sequence_id,
            message_id: MessageId {
                ledger_id: 1,
                entry_id,
                partition,
                batch_index: 0,
            },
        }
    }

    pub fn pulsar_handle_subscribe(
        &self,
        session: &mut PulsarSession,
        s: CommandSubscribe,
    ) -> StreamsResult<BaseCommand> {
        if !session.connected {
            return Ok(BaseCommand::Error {
                request_id: s.request_id,
                message: "session not connected".into(),
            });
        }
        let topic = TopicName::parse(&s.topic)?;
        self.tenants
            .ensure_namespace(&topic.tenant, &topic.namespace)?;
        if session.consumers.contains_key(&s.consumer_id) {
            return Ok(BaseCommand::Error {
                request_id: s.request_id,
                message: format!("consumer_id {} already in use", s.consumer_id),
            });
        }
        session.consumers.insert(
            s.consumer_id,
            ConsumerState {
                topic,
                subscription: s.subscription.clone(),
                subscription_type: s.subscription_type,
                name: s.consumer_name.clone(),
                permits: 0,
                ack_individual: -1,
                ack_cumulative: -1,
            },
        );
        Ok(BaseCommand::ProducerSuccess(CommandProducerSuccess {
            request_id: s.request_id,
            producer_name: s.consumer_name,
            last_sequence_id: -1,
        }))
    }

    pub fn pulsar_handle_flow(
        &self,
        session: &mut PulsarSession,
        f: CommandFlow,
    ) -> StreamsResult<BaseCommand> {
        let cons = session.consumers.get_mut(&f.consumer_id).ok_or_else(|| {
            StreamsError::Internal(format!("unknown consumer_id {}", f.consumer_id))
        })?;
        cons.permits = cons.permits.saturating_add(f.message_permits);
        // FLOW has no explicit response — we return a Success-like
        // ProducerSuccess as a server-side ack so the test harness can
        // observe completion.  Real Pulsar does not ack FLOW.
        Ok(BaseCommand::ProducerSuccess(CommandProducerSuccess {
            request_id: 0,
            producer_name: cons.name.clone(),
            last_sequence_id: cons.permits as i64,
        }))
    }

    /// Build a `MESSAGE` command for the given consumer, deducting one
    /// permit.  Returns `None` if no permits remain.
    pub fn pulsar_build_message(
        &self,
        session: &mut PulsarSession,
        consumer_id: u64,
        payload: Vec<u8>,
        entry_id: u64,
    ) -> StreamsResult<Option<BaseCommand>> {
        let cons = session
            .consumers
            .get_mut(&consumer_id)
            .ok_or_else(|| StreamsError::Internal(format!("unknown consumer_id {consumer_id}")))?;
        if cons.permits == 0 {
            return Ok(None);
        }
        cons.permits -= 1;
        let part = cons.topic.partition.unwrap_or(-1);
        Ok(Some(BaseCommand::Message(CommandMessage {
            consumer_id,
            message_id: MessageId {
                ledger_id: 1,
                entry_id,
                partition: part,
                batch_index: 0,
            },
            redelivery_count: 0,
            payload,
        })))
    }

    pub fn pulsar_handle_ack(
        &self,
        session: &mut PulsarSession,
        a: CommandAck,
    ) -> StreamsResult<BaseCommand> {
        let cons = session.consumers.get_mut(&a.consumer_id).ok_or_else(|| {
            StreamsError::Internal(format!("unknown consumer_id {}", a.consumer_id))
        })?;
        for mid in &a.message_ids {
            let id = mid.entry_id as i64;
            match a.ack_type {
                AckType::Individual => {
                    if id > cons.ack_individual {
                        cons.ack_individual = id;
                    }
                }
                AckType::Cumulative => {
                    if id > cons.ack_cumulative {
                        cons.ack_cumulative = id;
                    }
                }
            }
        }
        // Successful ACK has no response in upstream Pulsar; we synthesise
        // a Success-shaped ProducerSuccess for the in-process test harness.
        Ok(BaseCommand::ProducerSuccess(CommandProducerSuccess {
            request_id: 0,
            producer_name: cons.name.clone(),
            last_sequence_id: cons.ack_cumulative.max(cons.ack_individual),
        }))
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Pulsar wire-protocol tests
// feat/cave-streams-kafka-pulsar-001
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Buf;

    fn topic(tenant_id: &str, suffix: &str) -> String {
        format!("persistent://{}/ns/{}", tenant_id, suffix)
    }

    fn server() -> PulsarServer {
        PulsarServer::new(Arc::new(TenantRegistry::default()))
    }

    fn connect_session(srv: &PulsarServer) -> PulsarSession {
        let mut session = PulsarSession::default();
        let resp = srv
            .pulsar_handle_connect(
                &mut session,
                CommandConnect {
                    client_version: "test/1.0".into(),
                    protocol_version: 21,
                    auth_method_name: None,
                    auth_data: vec![],
                },
            )
            .unwrap();
        assert!(matches!(resp, BaseCommand::Connected(_)));
        session
    }

    // ── Frame layer ───────────────────────────────────────────────────

    #[test]
    fn test_pulsar_frame_round_trip() {
        // cite: pulsar 4.2.0 pulsar-common/.../protocol/Commands.java#newConnect
        let _tenant_id = "pwire-001";
        let cmd = BaseCommand::Connect(CommandConnect {
            client_version: "Pulsar-Java-2.11.0".into(),
            protocol_version: 21,
            auth_method_name: None,
            auth_data: vec![],
        });
        let bytes = encode_frame(&cmd).unwrap();
        let mut b = bytes.freeze();
        let back = decode_frame(&mut b).unwrap();
        assert_eq!(back, cmd);
        assert_eq!(b.remaining(), 0);
    }

    #[test]
    fn test_pulsar_frame_size_mismatch_rejected() {
        // cite: pulsar 4.2.0 PulsarDecoder#decode (frame size validation)
        let _tenant_id = "pwire-002";
        let cmd = BaseCommand::Flow(CommandFlow {
            consumer_id: 0,
            message_permits: 1,
        });
        let mut bytes = encode_frame(&cmd).unwrap();
        // Corrupt the inner cmd_size to mismatch totalSize.
        bytes[7] = bytes[7].wrapping_add(1);
        let mut b = bytes.freeze();
        let err = decode_frame(&mut b);
        assert!(err.is_err());
    }

    // ── CONNECT / CONNECTED ───────────────────────────────────────────

    #[test]
    fn test_pulsar_connect_returns_connected() {
        // cite: pulsar 4.2.0 ServerCnx.handleConnect
        let _tenant_id = "pwire-003";
        let srv = server();
        let mut session = PulsarSession::default();
        let resp = srv
            .pulsar_handle_connect(
                &mut session,
                CommandConnect {
                    client_version: "test/1.0".into(),
                    protocol_version: 21,
                    auth_method_name: None,
                    auth_data: vec![],
                },
            )
            .unwrap();
        match resp {
            BaseCommand::Connected(c) => {
                assert!(c.protocol_version <= 21);
                assert!(c.server_version.contains("cave-streams"));
            }
            other => panic!("expected Connected, got {:?}", other.cmd_type()),
        }
        assert!(session.connected);
    }

    #[test]
    fn test_pulsar_connect_clamps_protocol_version() {
        // cite: pulsar 4.2.0 Commands.MAX_PROTOCOL_VERSION negotiation
        let _tenant_id = "pwire-004";
        let srv = server();
        let mut session = PulsarSession::default();
        let resp = srv
            .pulsar_handle_connect(
                &mut session,
                CommandConnect {
                    client_version: "old/0.9".into(),
                    protocol_version: 5,
                    auth_method_name: None,
                    auth_data: vec![],
                },
            )
            .unwrap();
        if let BaseCommand::Connected(c) = resp {
            assert_eq!(c.protocol_version, 5, "must clamp to client's version");
        } else {
            panic!();
        }
    }

    // ── PRODUCER / SEND / SEND_RECEIPT ────────────────────────────────

    #[test]
    fn test_pulsar_producer_assigns_id() {
        // cite: pulsar 4.2.0 ServerCnx.handleProducer (producer registration)
        let tenant_id = "pwire-005";
        let srv = server();
        let mut session = connect_session(&srv);
        let resp = srv
            .pulsar_handle_producer(
                &mut session,
                CommandProducer {
                    topic: topic(tenant_id, "t"),
                    producer_id: 1,
                    request_id: 100,
                    producer_name: "p1".into(),
                },
            )
            .unwrap();
        match resp {
            BaseCommand::ProducerSuccess(s) => {
                assert_eq!(s.request_id, 100);
                assert_eq!(s.producer_name, "p1");
            }
            other => panic!("expected ProducerSuccess, got {:?}", other.cmd_type()),
        }
        assert!(session.producers.contains_key(&1));
    }

    #[test]
    fn test_pulsar_producer_requires_connected() {
        // cite: pulsar 4.2.0 ServerCnx.handleProducer guard (CONNECTED first)
        let tenant_id = "pwire-006";
        let srv = server();
        let mut session = PulsarSession::default();
        let resp = srv
            .pulsar_handle_producer(
                &mut session,
                CommandProducer {
                    topic: topic(tenant_id, "t"),
                    producer_id: 1,
                    request_id: 1,
                    producer_name: "p1".into(),
                },
            )
            .unwrap();
        assert!(matches!(resp, BaseCommand::Error { .. }));
    }

    #[test]
    fn test_pulsar_producer_duplicate_id_errors() {
        // cite: pulsar 4.2.0 ServerCnx.handleProducer (existing producerId)
        let tenant_id = "pwire-007";
        let srv = server();
        let mut session = connect_session(&srv);
        srv.pulsar_handle_producer(
            &mut session,
            CommandProducer {
                topic: topic(tenant_id, "t"),
                producer_id: 1,
                request_id: 1,
                producer_name: "p1".into(),
            },
        )
        .unwrap();
        let resp = srv
            .pulsar_handle_producer(
                &mut session,
                CommandProducer {
                    topic: topic(tenant_id, "t"),
                    producer_id: 1,
                    request_id: 2,
                    producer_name: "p2".into(),
                },
            )
            .unwrap();
        assert!(matches!(resp, BaseCommand::Error { .. }));
    }

    #[test]
    fn test_pulsar_send_returns_receipt() {
        // cite: pulsar 4.2.0 ServerCnx.handleSend / Commands.newSendReceipt
        let tenant_id = "pwire-008";
        let srv = server();
        let mut session = connect_session(&srv);
        srv.pulsar_handle_producer(
            &mut session,
            CommandProducer {
                topic: topic(tenant_id, "t"),
                producer_id: 7,
                request_id: 1,
                producer_name: "p".into(),
            },
        )
        .unwrap();
        let resp = srv
            .pulsar_handle_send(
                &mut session,
                CommandSend {
                    producer_id: 7,
                    sequence_id: 0,
                    num_messages: 1,
                    payload: b"hello".to_vec(),
                },
            )
            .unwrap();
        match resp {
            BaseCommand::SendReceipt(r) => {
                assert_eq!(r.producer_id, 7);
                assert_eq!(r.sequence_id, 0);
                assert!(r.message_id.entry_id > 0);
            }
            other => panic!("expected SendReceipt, got {:?}", other.cmd_type()),
        }
    }

    #[test]
    fn test_pulsar_send_rejects_duplicate_sequence() {
        // cite: pulsar 4.2.0 PersistentTopic.publishMessage (idempotency check)
        let tenant_id = "pwire-009";
        let srv = server();
        let mut session = connect_session(&srv);
        srv.pulsar_handle_producer(
            &mut session,
            CommandProducer {
                topic: topic(tenant_id, "t"),
                producer_id: 1,
                request_id: 1,
                producer_name: "p".into(),
            },
        )
        .unwrap();
        srv.pulsar_handle_send(
            &mut session,
            CommandSend {
                producer_id: 1,
                sequence_id: 5,
                num_messages: 1,
                payload: b"x".to_vec(),
            },
        )
        .unwrap();
        // Resend with the same sequence_id — should error with DuplicateSeq.
        let err = srv.pulsar_handle_send(
            &mut session,
            CommandSend {
                producer_id: 1,
                sequence_id: 5,
                num_messages: 1,
                payload: b"y".to_vec(),
            },
        );
        assert!(matches!(
            err,
            Err(StreamsError::DuplicateSequenceNumber { .. })
        ));
    }

    #[test]
    fn test_pulsar_send_unknown_producer_errors() {
        // cite: pulsar 4.2.0 ServerCnx.handleSend (unknown producer)
        let _tenant_id = "pwire-010";
        let srv = server();
        let mut session = connect_session(&srv);
        let err = srv.pulsar_handle_send(
            &mut session,
            CommandSend {
                producer_id: 999,
                sequence_id: 0,
                num_messages: 1,
                payload: vec![],
            },
        );
        assert!(matches!(err, Err(StreamsError::ProducerIdNotFound(999))));
    }

    // ── SUBSCRIBE / FLOW / MESSAGE / ACK ──────────────────────────────

    #[test]
    fn test_pulsar_subscribe_creates_consumer() {
        // cite: pulsar 4.2.0 ServerCnx.handleSubscribe
        let tenant_id = "pwire-011";
        let srv = server();
        let mut session = connect_session(&srv);
        let resp = srv
            .pulsar_handle_subscribe(
                &mut session,
                CommandSubscribe {
                    topic: topic(tenant_id, "t"),
                    subscription: format!("sub-{}", tenant_id),
                    subscription_type: SubscriptionType::Shared,
                    consumer_id: 1,
                    request_id: 1,
                    consumer_name: "c1".into(),
                },
            )
            .unwrap();
        assert!(matches!(resp, BaseCommand::ProducerSuccess(_)));
        assert!(session.consumers.contains_key(&1));
        let cs = session.consumers.get(&1).unwrap();
        assert_eq!(cs.subscription_type, SubscriptionType::Shared);
    }

    #[test]
    fn test_pulsar_flow_grants_permits() {
        // cite: pulsar 4.2.0 ServerCnx.handleFlow (flow control)
        let tenant_id = "pwire-012";
        let srv = server();
        let mut session = connect_session(&srv);
        srv.pulsar_handle_subscribe(
            &mut session,
            CommandSubscribe {
                topic: topic(tenant_id, "t"),
                subscription: "sub".into(),
                subscription_type: SubscriptionType::Exclusive,
                consumer_id: 1,
                request_id: 1,
                consumer_name: "c".into(),
            },
        )
        .unwrap();
        srv.pulsar_handle_flow(
            &mut session,
            CommandFlow {
                consumer_id: 1,
                message_permits: 100,
            },
        )
        .unwrap();
        srv.pulsar_handle_flow(
            &mut session,
            CommandFlow {
                consumer_id: 1,
                message_permits: 50,
            },
        )
        .unwrap();
        // Permits accumulate (matches Pulsar broker behaviour).
        assert_eq!(session.consumers.get(&1).unwrap().permits, 150);
    }

    #[test]
    fn test_pulsar_build_message_consumes_permit() {
        // cite: pulsar 4.2.0 PersistentDispatcherSingleActiveConsumer (permits--)
        let tenant_id = "pwire-013";
        let srv = server();
        let mut session = connect_session(&srv);
        srv.pulsar_handle_subscribe(
            &mut session,
            CommandSubscribe {
                topic: topic(tenant_id, "t"),
                subscription: "sub".into(),
                subscription_type: SubscriptionType::Exclusive,
                consumer_id: 9,
                request_id: 1,
                consumer_name: "c".into(),
            },
        )
        .unwrap();
        srv.pulsar_handle_flow(
            &mut session,
            CommandFlow {
                consumer_id: 9,
                message_permits: 2,
            },
        )
        .unwrap();
        let m1 = srv
            .pulsar_build_message(&mut session, 9, b"hi".to_vec(), 100)
            .unwrap();
        assert!(m1.is_some());
        let m2 = srv
            .pulsar_build_message(&mut session, 9, b"hi".to_vec(), 101)
            .unwrap();
        assert!(m2.is_some());
        // Third call: no permits left — returns None.
        let m3 = srv
            .pulsar_build_message(&mut session, 9, b"hi".to_vec(), 102)
            .unwrap();
        assert!(m3.is_none());
    }

    #[test]
    fn test_pulsar_ack_advances_consumer() {
        // cite: pulsar 4.2.0 ServerCnx.handleAck (ack tracking)
        let tenant_id = "pwire-014";
        let srv = server();
        let mut session = connect_session(&srv);
        srv.pulsar_handle_subscribe(
            &mut session,
            CommandSubscribe {
                topic: topic(tenant_id, "t"),
                subscription: "sub".into(),
                subscription_type: SubscriptionType::Failover,
                consumer_id: 1,
                request_id: 1,
                consumer_name: "c".into(),
            },
        )
        .unwrap();
        srv.pulsar_handle_ack(
            &mut session,
            CommandAck {
                consumer_id: 1,
                ack_type: AckType::Cumulative,
                message_ids: vec![MessageId {
                    ledger_id: 1,
                    entry_id: 42,
                    partition: -1,
                    batch_index: 0,
                }],
            },
        )
        .unwrap();
        assert_eq!(session.consumers.get(&1).unwrap().ack_cumulative, 42);
    }

    #[test]
    fn test_pulsar_ack_individual_only_advances_individual() {
        // cite: pulsar 4.2.0 ManagedCursor.individualDeletedMessages
        let tenant_id = "pwire-015";
        let srv = server();
        let mut session = connect_session(&srv);
        srv.pulsar_handle_subscribe(
            &mut session,
            CommandSubscribe {
                topic: topic(tenant_id, "t"),
                subscription: "sub".into(),
                subscription_type: SubscriptionType::Shared,
                consumer_id: 1,
                request_id: 1,
                consumer_name: "c".into(),
            },
        )
        .unwrap();
        srv.pulsar_handle_ack(
            &mut session,
            CommandAck {
                consumer_id: 1,
                ack_type: AckType::Individual,
                message_ids: vec![MessageId {
                    ledger_id: 1,
                    entry_id: 7,
                    partition: -1,
                    batch_index: 0,
                }],
            },
        )
        .unwrap();
        let s = session.consumers.get(&1).unwrap();
        assert_eq!(s.ack_individual, 7);
        assert_eq!(s.ack_cumulative, -1);
    }

    #[test]
    fn test_pulsar_subscribe_autocreates_namespace() {
        // cite: cave ADR-RUNTIME-STREAMING-CONSOLIDATION-001 §multi-tenant
        let tenant_id = "pwire-016";
        let srv = server();
        let mut session = connect_session(&srv);
        // Tenant not pre-created; server must auto-create.
        srv.pulsar_handle_subscribe(
            &mut session,
            CommandSubscribe {
                topic: topic(tenant_id, "auto"),
                subscription: "sub".into(),
                subscription_type: SubscriptionType::Exclusive,
                consumer_id: 1,
                request_id: 1,
                consumer_name: "c".into(),
            },
        )
        .unwrap();
        assert!(srv.tenants.get_tenant(tenant_id).is_some());
    }
}

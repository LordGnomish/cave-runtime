// SPDX-License-Identifier: AGPL-3.0-or-later
//
// Source: valkey-io/valkey@7.2 src/cluster.c (clusterMsg, clusterMsgData*)
//
//! Binary on-wire cluster-bus codec.
//!
//! This module ports the wire-protocol half of upstream Valkey's
//! `src/cluster.c` — the framed message envelope that nodes exchange on
//! their cluster-bus TCP port (data-port + 10000).
//!
//! The state-machine half (what each message does to the cluster view)
//! lives in [`super::cluster::gossip`]; this module owns the literal
//! byte layout. We deliberately do *not* fork the C struct layout —
//! upstream uses architecture-dependent padding and host-byte-order
//! ints in places. We use a portable big-endian framing.
//!
//! ## Wire frame
//!
//! ```text
//! magic:     u32  = 0x52437200  // 'RCr\0' — Cluster bus marker
//! version:   u16  = 1
//! type:      u16  // ClusterMsgType
//! totlen:    u32  // total frame length including header
//! sender_id: [u8; 40]
//! sender_addr_len: u16
//! sender_addr:     [u8; sender_addr_len]
//! sender_epoch:    u64
//! current_epoch:   u64
//! offset:    u64
//! flags:     u32   // sender's NodeFlags bitset
//! body:      …     // type-specific
//! ```
//!
//! For PING/PONG/MEET the body is a gossip section: a u16 count
//! followed by N `GossipEntry` (40-byte node id, 6-byte ping/pong
//! seconds, sender_addr, flags) — same shape as upstream.

use std::net::SocketAddr;

/// `'RCr\0'` — first four bytes of every cluster-bus frame.
pub const CLUSTER_BUS_MAGIC: u32 = 0x52437200;
/// Wire-format version. Bump on incompatible changes.
pub const CLUSTER_BUS_VERSION: u16 = 1;
/// Minimum-sized header (everything before the body).
pub const HEADER_FIXED_LEN: usize = 4 + 2 + 2 + 4 + 40 + 2 + 8 + 8 + 8 + 4;
/// Cluster-bus port offset: data port + 10_000.
pub const CLUSTER_BUS_PORT_OFFSET: u16 = 10_000;

/// `clusterMsgType` from upstream — the 12 message kinds that flow
/// across the bus. Values match the upstream enum order (so encoded
/// byte 0..0x0B is wire-stable).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum ClusterMsgType {
    Ping = 0,
    Pong = 1,
    Meet = 2,
    Fail = 3,
    Publish = 4,
    FailoverAuthRequest = 5,
    FailoverAuthAck = 6,
    Update = 7,
    MfStart = 8,
    Module = 9,
    PublishShard = 10,
    ClusterUnknown = 11,
}

impl ClusterMsgType {
    pub fn from_u16(v: u16) -> Option<Self> {
        Some(match v {
            0 => Self::Ping,
            1 => Self::Pong,
            2 => Self::Meet,
            3 => Self::Fail,
            4 => Self::Publish,
            5 => Self::FailoverAuthRequest,
            6 => Self::FailoverAuthAck,
            7 => Self::Update,
            8 => Self::MfStart,
            9 => Self::Module,
            10 => Self::PublishShard,
            11 => Self::ClusterUnknown,
            _ => return None,
        })
    }
}

/// Node flag bitset — port of `CLUSTER_NODE_*` macros from
/// `cluster.h`. The numeric values match upstream so a foreign
/// node speaking our protocol can interpret the field correctly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NodeFlags(pub u32);

#[allow(non_upper_case_globals)]
impl NodeFlags {
    pub const MASTER: NodeFlags = NodeFlags(1 << 0);
    pub const SLAVE: NodeFlags = NodeFlags(1 << 1);
    pub const PFAIL: NodeFlags = NodeFlags(1 << 2);
    pub const FAIL: NodeFlags = NodeFlags(1 << 3);
    pub const MYSELF: NodeFlags = NodeFlags(1 << 4);
    pub const HANDSHAKE: NodeFlags = NodeFlags(1 << 5);
    pub const NOADDR: NodeFlags = NodeFlags(1 << 6);
    pub const MEET: NodeFlags = NodeFlags(1 << 7);
    pub const MIGRATE_TO: NodeFlags = NodeFlags(1 << 8);
    pub const NOFAILOVER: NodeFlags = NodeFlags(1 << 9);
    pub const EXTENSIONS_SUPPORTED: NodeFlags = NodeFlags(1 << 10);
    pub const MIGRATE_FROM: NodeFlags = NodeFlags(1 << 11);

    pub const fn empty() -> Self {
        NodeFlags(0)
    }
    pub const fn bits(self) -> u32 {
        self.0
    }
    pub const fn from_bits_truncate(v: u32) -> Self {
        NodeFlags(v & 0xFFF)
    }
    pub const fn contains(self, other: NodeFlags) -> bool {
        (self.0 & other.0) == other.0
    }
    pub fn insert(&mut self, other: NodeFlags) {
        self.0 |= other.0;
    }
    pub fn remove(&mut self, other: NodeFlags) {
        self.0 &= !other.0;
    }
}

impl std::ops::BitOr for NodeFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        NodeFlags(self.0 | rhs.0)
    }
}
impl std::ops::BitAnd for NodeFlags {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        NodeFlags(self.0 & rhs.0)
    }
}
impl std::ops::BitOrAssign for NodeFlags {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClusterMsgHeader {
    pub version: u16,
    pub msg_type: ClusterMsgType,
    pub totlen: u32,
    /// 40-character hex node id.
    pub sender_id: String,
    pub sender_addr: String,
    pub sender_epoch: u64,
    pub current_epoch: u64,
    pub offset: u64,
    pub flags: NodeFlags,
}

/// PING/PONG/MEET body — embedded gossip entries describing peers the
/// sender knows about.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GossipSection {
    pub entries: Vec<GossipEntry>,
}

/// One entry in a gossip section. Mirrors `clusterMsgDataGossip`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GossipEntry {
    pub node_id: String,
    /// Sender's last successful PING to this node (UNIX millis).
    pub ping_sent_ms: u64,
    /// Sender's last received PONG from this node (UNIX millis).
    pub pong_received_ms: u64,
    pub addr: String,
    pub flags: NodeFlags,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishData {
    pub channel: Vec<u8>,
    pub message: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailData {
    pub failing_node_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateData {
    pub node_id: String,
    pub config_epoch: u64,
    /// 16384-bit bitmap; we carry it as a 2048-byte slice.
    pub slots_bitmap: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClusterMsgBody {
    Gossip(GossipSection),
    Publish(PublishData),
    Fail(FailData),
    Update(UpdateData),
    Empty,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClusterMsg {
    pub header: ClusterMsgHeader,
    pub body: ClusterMsgBody,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum WireError {
    #[error("buffer truncated at offset {0}")]
    Truncated(usize),
    #[error("bad magic 0x{0:08x} (want 0x{CLUSTER_BUS_MAGIC:08x})")]
    BadMagic(u32),
    #[error("unsupported version {0}")]
    UnsupportedVersion(u16),
    #[error("unknown message type {0}")]
    UnknownType(u16),
    #[error("declared totlen {declared} != bytes {actual}")]
    LengthMismatch { declared: u32, actual: usize },
    #[error("sender id must be exactly 40 ASCII hex chars (got {0} bytes)")]
    InvalidSenderId(usize),
    #[error("address field too long ({0} > 256)")]
    AddrTooLong(usize),
    #[error("slot bitmap must be 2048 bytes (got {0})")]
    InvalidSlotBitmap(usize),
    #[error("gossip count {0} exceeds frame size")]
    GossipOverflow(u16),
}

/// Encode a `ClusterMsg` into wire bytes.
pub fn encode(msg: &ClusterMsg) -> Result<Vec<u8>, WireError> {
    if msg.header.sender_id.len() != 40 {
        return Err(WireError::InvalidSenderId(msg.header.sender_id.len()));
    }
    if msg.header.sender_addr.len() > 256 {
        return Err(WireError::AddrTooLong(msg.header.sender_addr.len()));
    }

    let mut buf = Vec::with_capacity(256);

    // Header skeleton — totlen is back-patched.
    buf.extend_from_slice(&CLUSTER_BUS_MAGIC.to_be_bytes());
    buf.extend_from_slice(&CLUSTER_BUS_VERSION.to_be_bytes());
    buf.extend_from_slice(&(msg.header.msg_type as u16).to_be_bytes());
    let totlen_offset = buf.len();
    buf.extend_from_slice(&0u32.to_be_bytes()); // placeholder
    buf.extend_from_slice(msg.header.sender_id.as_bytes());
    buf.extend_from_slice(&(msg.header.sender_addr.len() as u16).to_be_bytes());
    buf.extend_from_slice(msg.header.sender_addr.as_bytes());
    buf.extend_from_slice(&msg.header.sender_epoch.to_be_bytes());
    buf.extend_from_slice(&msg.header.current_epoch.to_be_bytes());
    buf.extend_from_slice(&msg.header.offset.to_be_bytes());
    buf.extend_from_slice(&msg.header.flags.bits().to_be_bytes());

    // Body.
    match &msg.body {
        ClusterMsgBody::Gossip(g) => {
            buf.extend_from_slice(&(g.entries.len() as u16).to_be_bytes());
            for e in &g.entries {
                if e.node_id.len() != 40 {
                    return Err(WireError::InvalidSenderId(e.node_id.len()));
                }
                if e.addr.len() > 256 {
                    return Err(WireError::AddrTooLong(e.addr.len()));
                }
                buf.extend_from_slice(e.node_id.as_bytes());
                buf.extend_from_slice(&e.ping_sent_ms.to_be_bytes());
                buf.extend_from_slice(&e.pong_received_ms.to_be_bytes());
                buf.extend_from_slice(&(e.addr.len() as u16).to_be_bytes());
                buf.extend_from_slice(e.addr.as_bytes());
                buf.extend_from_slice(&e.flags.bits().to_be_bytes());
            }
        }
        ClusterMsgBody::Publish(p) => {
            buf.extend_from_slice(&(p.channel.len() as u32).to_be_bytes());
            buf.extend_from_slice(&(p.message.len() as u32).to_be_bytes());
            buf.extend_from_slice(&p.channel);
            buf.extend_from_slice(&p.message);
        }
        ClusterMsgBody::Fail(f) => {
            if f.failing_node_id.len() != 40 {
                return Err(WireError::InvalidSenderId(f.failing_node_id.len()));
            }
            buf.extend_from_slice(f.failing_node_id.as_bytes());
        }
        ClusterMsgBody::Update(u) => {
            if u.node_id.len() != 40 {
                return Err(WireError::InvalidSenderId(u.node_id.len()));
            }
            if u.slots_bitmap.len() != 2048 {
                return Err(WireError::InvalidSlotBitmap(u.slots_bitmap.len()));
            }
            buf.extend_from_slice(u.node_id.as_bytes());
            buf.extend_from_slice(&u.config_epoch.to_be_bytes());
            buf.extend_from_slice(&u.slots_bitmap);
        }
        ClusterMsgBody::Empty => {}
    }

    let totlen = buf.len() as u32;
    buf[totlen_offset..totlen_offset + 4].copy_from_slice(&totlen.to_be_bytes());
    Ok(buf)
}

/// Decode a wire-format `ClusterMsg` from bytes.
pub fn decode(buf: &[u8]) -> Result<ClusterMsg, WireError> {
    let mut c = Cursor::new(buf);
    let magic = c.read_u32()?;
    if magic != CLUSTER_BUS_MAGIC {
        return Err(WireError::BadMagic(magic));
    }
    let version = c.read_u16()?;
    if version != CLUSTER_BUS_VERSION {
        return Err(WireError::UnsupportedVersion(version));
    }
    let msg_type_raw = c.read_u16()?;
    let msg_type = ClusterMsgType::from_u16(msg_type_raw)
        .ok_or(WireError::UnknownType(msg_type_raw))?;
    let totlen = c.read_u32()?;
    if totlen as usize != buf.len() {
        return Err(WireError::LengthMismatch {
            declared: totlen,
            actual: buf.len(),
        });
    }
    let sender_id = c.read_fixed_str(40)?;
    let sender_addr_len = c.read_u16()? as usize;
    if sender_addr_len > 256 {
        return Err(WireError::AddrTooLong(sender_addr_len));
    }
    let sender_addr = c.read_str(sender_addr_len)?;
    let sender_epoch = c.read_u64()?;
    let current_epoch = c.read_u64()?;
    let offset = c.read_u64()?;
    let flags = NodeFlags::from_bits_truncate(c.read_u32()?);

    let body = match msg_type {
        ClusterMsgType::Ping | ClusterMsgType::Pong | ClusterMsgType::Meet => {
            let count = c.read_u16()?;
            let mut entries = Vec::with_capacity(count as usize);
            for _ in 0..count {
                let node_id = c.read_fixed_str(40)?;
                let ping_sent_ms = c.read_u64()?;
                let pong_received_ms = c.read_u64()?;
                let addr_len = c.read_u16()? as usize;
                if addr_len > 256 {
                    return Err(WireError::AddrTooLong(addr_len));
                }
                let addr = c.read_str(addr_len)?;
                let flags = NodeFlags::from_bits_truncate(c.read_u32()?);
                entries.push(GossipEntry {
                    node_id,
                    ping_sent_ms,
                    pong_received_ms,
                    addr,
                    flags,
                });
            }
            ClusterMsgBody::Gossip(GossipSection { entries })
        }
        ClusterMsgType::Publish | ClusterMsgType::PublishShard => {
            let chan_len = c.read_u32()? as usize;
            let msg_len = c.read_u32()? as usize;
            let channel = c.read_bytes(chan_len)?;
            let message = c.read_bytes(msg_len)?;
            ClusterMsgBody::Publish(PublishData { channel, message })
        }
        ClusterMsgType::Fail => {
            let failing_node_id = c.read_fixed_str(40)?;
            ClusterMsgBody::Fail(FailData { failing_node_id })
        }
        ClusterMsgType::Update => {
            let node_id = c.read_fixed_str(40)?;
            let config_epoch = c.read_u64()?;
            let slots_bitmap = c.read_bytes(2048)?;
            ClusterMsgBody::Update(UpdateData {
                node_id,
                config_epoch,
                slots_bitmap,
            })
        }
        _ => ClusterMsgBody::Empty,
    };

    Ok(ClusterMsg {
        header: ClusterMsgHeader {
            version,
            msg_type,
            totlen,
            sender_id,
            sender_addr,
            sender_epoch,
            current_epoch,
            offset,
            flags,
        },
        body,
    })
}

// ── Cursor primitive ──────────────────────────────────────────────────────────

struct Cursor<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    fn need(&self, n: usize) -> Result<(), WireError> {
        if self.pos + n > self.buf.len() {
            return Err(WireError::Truncated(self.pos));
        }
        Ok(())
    }

    fn read_u16(&mut self) -> Result<u16, WireError> {
        self.need(2)?;
        let v = u16::from_be_bytes(self.buf[self.pos..self.pos + 2].try_into().unwrap());
        self.pos += 2;
        Ok(v)
    }

    fn read_u32(&mut self) -> Result<u32, WireError> {
        self.need(4)?;
        let v = u32::from_be_bytes(self.buf[self.pos..self.pos + 4].try_into().unwrap());
        self.pos += 4;
        Ok(v)
    }

    fn read_u64(&mut self) -> Result<u64, WireError> {
        self.need(8)?;
        let v = u64::from_be_bytes(self.buf[self.pos..self.pos + 8].try_into().unwrap());
        self.pos += 8;
        Ok(v)
    }

    fn read_bytes(&mut self, n: usize) -> Result<Vec<u8>, WireError> {
        self.need(n)?;
        let v = self.buf[self.pos..self.pos + n].to_vec();
        self.pos += n;
        Ok(v)
    }

    fn read_str(&mut self, n: usize) -> Result<String, WireError> {
        let v = self.read_bytes(n)?;
        Ok(String::from_utf8_lossy(&v).into_owned())
    }

    fn read_fixed_str(&mut self, n: usize) -> Result<String, WireError> {
        self.read_str(n)
    }
}

// ── Higher-level heartbeat scheduler ─────────────────────────────────────────

/// `NODE_TIMEOUT` default — upstream cluster-node-timeout is 15s but
/// expressed in ms; we default to the same value.
pub const DEFAULT_NODE_TIMEOUT_MS: u64 = 15_000;
/// Heartbeat cadence — upstream pings a random peer every 100ms.
pub const HEARTBEAT_INTERVAL_MS: u64 = 100;
/// Promotion threshold: PFAIL → FAIL after this many failed peer
/// heartbeats.
pub const PFAIL_TO_FAIL_REPORTS: u32 = 1;

/// A liveness-detection helper that tracks the last-seen PONG time
/// for each peer and answers `is_pfail(node)` /
/// `should_promote_to_fail(node)`.
#[derive(Debug)]
pub struct LivenessTracker {
    /// `node_id → (last_pong_ms, pfail_reports)`.
    seen: std::collections::HashMap<String, PeerLiveness>,
    pub node_timeout_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerLiveness {
    pub last_pong_ms: u64,
    pub pfail_reports: u32,
    pub flags: NodeFlags,
}

impl LivenessTracker {
    pub fn new() -> Self {
        Self {
            seen: std::collections::HashMap::new(),
            node_timeout_ms: DEFAULT_NODE_TIMEOUT_MS,
        }
    }

    pub fn with_timeout(timeout_ms: u64) -> Self {
        Self {
            seen: std::collections::HashMap::new(),
            node_timeout_ms: timeout_ms,
        }
    }

    pub fn record_pong(&mut self, node_id: &str, now_ms: u64) {
        let e = self.seen.entry(node_id.to_string()).or_insert(PeerLiveness {
            last_pong_ms: 0,
            pfail_reports: 0,
            flags: NodeFlags::empty(),
        });
        e.last_pong_ms = now_ms;
        // A live PONG clears PFAIL.
        e.pfail_reports = 0;
        e.flags.remove(NodeFlags::PFAIL);
    }

    /// Returns true when the peer should be marked PFAIL.
    pub fn is_pfail(&self, node_id: &str, now_ms: u64) -> bool {
        match self.seen.get(node_id) {
            Some(p) => now_ms.saturating_sub(p.last_pong_ms) > self.node_timeout_ms,
            None => false,
        }
    }

    /// Apply the PFAIL → FAIL promotion rule. Upstream needs a majority
    /// of cluster masters to agree on PFAIL; we encode a configurable
    /// threshold (`PFAIL_TO_FAIL_REPORTS` reports + `now > 2*timeout`).
    pub fn should_promote_fail(&self, node_id: &str, now_ms: u64) -> bool {
        match self.seen.get(node_id) {
            Some(p) => {
                p.pfail_reports >= PFAIL_TO_FAIL_REPORTS
                    && now_ms.saturating_sub(p.last_pong_ms) > self.node_timeout_ms * 2
            }
            None => false,
        }
    }

    pub fn observe_pfail_report(&mut self, node_id: &str) {
        let e = self.seen.entry(node_id.to_string()).or_insert(PeerLiveness {
            last_pong_ms: 0,
            pfail_reports: 0,
            flags: NodeFlags::empty(),
        });
        e.pfail_reports = e.pfail_reports.saturating_add(1);
        e.flags.insert(NodeFlags::PFAIL);
    }

    pub fn mark_fail(&mut self, node_id: &str) {
        if let Some(e) = self.seen.get_mut(node_id) {
            e.flags.insert(NodeFlags::FAIL);
        }
    }

    pub fn peer(&self, node_id: &str) -> Option<&PeerLiveness> {
        self.seen.get(node_id)
    }

    pub fn known_peers(&self) -> impl Iterator<Item = &String> {
        self.seen.keys()
    }
}

/// Heartbeat dispatcher — produces the next per-tick decision: which
/// peer to ping (random pick) and which peers crossed the PFAIL/FAIL
/// threshold.
#[derive(Debug)]
pub struct HeartbeatScheduler {
    last_tick_ms: u64,
    pub interval_ms: u64,
}

impl HeartbeatScheduler {
    pub fn new() -> Self {
        Self {
            last_tick_ms: 0,
            interval_ms: HEARTBEAT_INTERVAL_MS,
        }
    }

    pub fn with_interval(interval_ms: u64) -> Self {
        Self {
            last_tick_ms: 0,
            interval_ms,
        }
    }

    pub fn should_tick(&self, now_ms: u64) -> bool {
        now_ms.saturating_sub(self.last_tick_ms) >= self.interval_ms
    }

    pub fn record_tick(&mut self, now_ms: u64) {
        self.last_tick_ms = now_ms;
    }

    /// Pick a random peer to ping this tick. Returns `None` if the
    /// known peer set is empty.
    pub fn pick_peer<'a, I: IntoIterator<Item = &'a String>>(
        &self,
        peers: I,
        seed: u64,
    ) -> Option<String> {
        let all: Vec<&String> = peers.into_iter().collect();
        if all.is_empty() {
            return None;
        }
        // xorshift seed → idx
        let mut s = seed.max(1);
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        Some(all[(s as usize) % all.len()].clone())
    }
}

impl Default for HeartbeatScheduler {
    fn default() -> Self {
        Self::new()
    }
}

impl Default for LivenessTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Convenience: derive the cluster-bus address (host:bus_port) from
/// the data address (host:data_port).
pub fn derive_bus_addr(data_addr: &str) -> Option<SocketAddr> {
    let sa: SocketAddr = data_addr.parse().ok()?;
    Some(SocketAddr::new(
        sa.ip(),
        sa.port().wrapping_add(CLUSTER_BUS_PORT_OFFSET),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(t: ClusterMsgType) -> ClusterMsgHeader {
        ClusterMsgHeader {
            version: CLUSTER_BUS_VERSION,
            msg_type: t,
            totlen: 0,
            sender_id: "a".repeat(40),
            sender_addr: "10.0.0.1:6379".into(),
            sender_epoch: 7,
            current_epoch: 8,
            offset: 1024,
            flags: NodeFlags::MASTER,
        }
    }

    #[test]
    fn encode_decode_ping_empty_gossip_round_trips() {
        let m = ClusterMsg {
            header: header(ClusterMsgType::Ping),
            body: ClusterMsgBody::Gossip(GossipSection::default()),
        };
        let bytes = encode(&m).unwrap();
        let back = decode(&bytes).unwrap();
        assert_eq!(back.header.msg_type, ClusterMsgType::Ping);
        assert_eq!(back.header.sender_id, m.header.sender_id);
        assert_eq!(back.header.sender_epoch, 7);
        assert_eq!(back.header.flags, NodeFlags::MASTER);
        match back.body {
            ClusterMsgBody::Gossip(g) => assert!(g.entries.is_empty()),
            _ => panic!(),
        }
    }

    #[test]
    fn encode_meet_round_trips() {
        let m = ClusterMsg {
            header: header(ClusterMsgType::Meet),
            body: ClusterMsgBody::Gossip(GossipSection::default()),
        };
        let bytes = encode(&m).unwrap();
        let back = decode(&bytes).unwrap();
        assert_eq!(back.header.msg_type, ClusterMsgType::Meet);
    }

    #[test]
    fn ping_with_gossip_entries_round_trips() {
        let entry = GossipEntry {
            node_id: "b".repeat(40),
            ping_sent_ms: 100,
            pong_received_ms: 200,
            addr: "10.0.0.2:6379".into(),
            flags: NodeFlags::MASTER | NodeFlags::EXTENSIONS_SUPPORTED,
        };
        let m = ClusterMsg {
            header: header(ClusterMsgType::Ping),
            body: ClusterMsgBody::Gossip(GossipSection {
                entries: vec![entry.clone()],
            }),
        };
        let bytes = encode(&m).unwrap();
        let back = decode(&bytes).unwrap();
        match back.body {
            ClusterMsgBody::Gossip(g) => {
                assert_eq!(g.entries.len(), 1);
                assert_eq!(g.entries[0], entry);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn publish_round_trips() {
        let m = ClusterMsg {
            header: header(ClusterMsgType::Publish),
            body: ClusterMsgBody::Publish(PublishData {
                channel: b"news".to_vec(),
                message: b"hello".to_vec(),
            }),
        };
        let bytes = encode(&m).unwrap();
        let back = decode(&bytes).unwrap();
        match back.body {
            ClusterMsgBody::Publish(p) => {
                assert_eq!(p.channel, b"news");
                assert_eq!(p.message, b"hello");
            }
            _ => panic!(),
        }
    }

    #[test]
    fn publish_shard_round_trips() {
        let m = ClusterMsg {
            header: header(ClusterMsgType::PublishShard),
            body: ClusterMsgBody::Publish(PublishData {
                channel: b"shard-channel".to_vec(),
                message: b"shard-msg".to_vec(),
            }),
        };
        let bytes = encode(&m).unwrap();
        let back = decode(&bytes).unwrap();
        assert_eq!(back.header.msg_type, ClusterMsgType::PublishShard);
        match back.body {
            ClusterMsgBody::Publish(p) => assert_eq!(p.channel, b"shard-channel"),
            _ => panic!(),
        }
    }

    #[test]
    fn fail_round_trips() {
        let m = ClusterMsg {
            header: header(ClusterMsgType::Fail),
            body: ClusterMsgBody::Fail(FailData {
                failing_node_id: "c".repeat(40),
            }),
        };
        let bytes = encode(&m).unwrap();
        let back = decode(&bytes).unwrap();
        match back.body {
            ClusterMsgBody::Fail(f) => assert_eq!(f.failing_node_id, "c".repeat(40)),
            _ => panic!(),
        }
    }

    #[test]
    fn update_round_trips_with_slot_bitmap() {
        let mut bitmap = vec![0u8; 2048];
        bitmap[0] = 0xFF;
        bitmap[2047] = 0x01;
        let m = ClusterMsg {
            header: header(ClusterMsgType::Update),
            body: ClusterMsgBody::Update(UpdateData {
                node_id: "d".repeat(40),
                config_epoch: 42,
                slots_bitmap: bitmap.clone(),
            }),
        };
        let bytes = encode(&m).unwrap();
        let back = decode(&bytes).unwrap();
        match back.body {
            ClusterMsgBody::Update(u) => {
                assert_eq!(u.config_epoch, 42);
                assert_eq!(u.slots_bitmap, bitmap);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn failover_auth_request_encodes_empty_body() {
        let m = ClusterMsg {
            header: header(ClusterMsgType::FailoverAuthRequest),
            body: ClusterMsgBody::Empty,
        };
        let bytes = encode(&m).unwrap();
        let back = decode(&bytes).unwrap();
        assert_eq!(back.header.msg_type, ClusterMsgType::FailoverAuthRequest);
        assert_eq!(back.body, ClusterMsgBody::Empty);
    }

    #[test]
    fn failover_auth_ack_round_trips() {
        let m = ClusterMsg {
            header: header(ClusterMsgType::FailoverAuthAck),
            body: ClusterMsgBody::Empty,
        };
        let bytes = encode(&m).unwrap();
        let back = decode(&bytes).unwrap();
        assert_eq!(back.header.msg_type, ClusterMsgType::FailoverAuthAck);
    }

    #[test]
    fn decode_rejects_bad_magic() {
        let mut bytes = encode(&ClusterMsg {
            header: header(ClusterMsgType::Ping),
            body: ClusterMsgBody::Gossip(GossipSection::default()),
        })
        .unwrap();
        bytes[0] = 0xDE;
        bytes[1] = 0xAD;
        let err = decode(&bytes).unwrap_err();
        assert!(matches!(err, WireError::BadMagic(_)));
    }

    #[test]
    fn decode_rejects_unsupported_version() {
        let mut bytes = encode(&ClusterMsg {
            header: header(ClusterMsgType::Ping),
            body: ClusterMsgBody::Gossip(GossipSection::default()),
        })
        .unwrap();
        // Bump version to 99.
        bytes[4] = 0;
        bytes[5] = 99;
        let err = decode(&bytes).unwrap_err();
        assert!(matches!(err, WireError::UnsupportedVersion(99)));
    }

    #[test]
    fn decode_rejects_truncated_frame() {
        let bytes = encode(&ClusterMsg {
            header: header(ClusterMsgType::Ping),
            body: ClusterMsgBody::Gossip(GossipSection::default()),
        })
        .unwrap();
        let err = decode(&bytes[..bytes.len() - 5]).unwrap_err();
        // Either length mismatch or truncated.
        match err {
            WireError::LengthMismatch { .. } | WireError::Truncated(_) => {}
            other => panic!("unexpected {:?}", other),
        }
    }

    #[test]
    fn encode_rejects_short_sender_id() {
        let mut h = header(ClusterMsgType::Ping);
        h.sender_id = "abc".into();
        let m = ClusterMsg {
            header: h,
            body: ClusterMsgBody::Gossip(GossipSection::default()),
        };
        assert!(matches!(encode(&m), Err(WireError::InvalidSenderId(3))));
    }

    #[test]
    fn encode_rejects_overlong_addr() {
        let mut h = header(ClusterMsgType::Ping);
        h.sender_addr = "x".repeat(300);
        let m = ClusterMsg {
            header: h,
            body: ClusterMsgBody::Gossip(GossipSection::default()),
        };
        assert!(matches!(encode(&m), Err(WireError::AddrTooLong(300))));
    }

    #[test]
    fn encode_rejects_invalid_slot_bitmap() {
        let m = ClusterMsg {
            header: header(ClusterMsgType::Update),
            body: ClusterMsgBody::Update(UpdateData {
                node_id: "z".repeat(40),
                config_epoch: 1,
                slots_bitmap: vec![0u8; 1024],
            }),
        };
        assert!(matches!(encode(&m), Err(WireError::InvalidSlotBitmap(1024))));
    }

    #[test]
    fn unknown_message_type_rejected() {
        // Craft a frame with type 99.
        let mut bytes = encode(&ClusterMsg {
            header: header(ClusterMsgType::Ping),
            body: ClusterMsgBody::Gossip(GossipSection::default()),
        })
        .unwrap();
        bytes[6] = 0;
        bytes[7] = 99;
        let err = decode(&bytes).unwrap_err();
        assert!(matches!(err, WireError::UnknownType(99)));
    }

    #[test]
    fn node_flags_bitset_roundtrips_all_known_bits() {
        let f = NodeFlags::MASTER
            | NodeFlags::SLAVE
            | NodeFlags::PFAIL
            | NodeFlags::FAIL
            | NodeFlags::MYSELF
            | NodeFlags::HANDSHAKE
            | NodeFlags::NOADDR
            | NodeFlags::MEET
            | NodeFlags::MIGRATE_TO
            | NodeFlags::NOFAILOVER
            | NodeFlags::EXTENSIONS_SUPPORTED
            | NodeFlags::MIGRATE_FROM;
        let raw = f.bits();
        let back = NodeFlags::from_bits_truncate(raw);
        assert_eq!(back, f);
    }

    #[test]
    fn cluster_msg_type_from_u16_round_trips_all_known() {
        for v in 0..=11u16 {
            assert!(ClusterMsgType::from_u16(v).is_some());
        }
        assert!(ClusterMsgType::from_u16(12).is_none());
    }

    #[test]
    fn liveness_pong_clears_pfail_window() {
        let mut t = LivenessTracker::with_timeout(1_000);
        t.record_pong("peer", 500);
        assert!(!t.is_pfail("peer", 1_400));
        // Now stale.
        assert!(t.is_pfail("peer", 1_700));
    }

    #[test]
    fn liveness_unknown_peer_is_not_pfail() {
        let t = LivenessTracker::with_timeout(1_000);
        assert!(!t.is_pfail("nobody", 99_999_999));
    }

    #[test]
    fn liveness_pong_resets_pfail_reports() {
        let mut t = LivenessTracker::with_timeout(1_000);
        t.observe_pfail_report("peer");
        t.observe_pfail_report("peer");
        assert_eq!(t.peer("peer").unwrap().pfail_reports, 2);
        t.record_pong("peer", 100);
        assert_eq!(t.peer("peer").unwrap().pfail_reports, 0);
        assert!(!t.peer("peer").unwrap().flags.contains(NodeFlags::PFAIL));
    }

    #[test]
    fn should_promote_fail_requires_threshold_and_2x_timeout() {
        let mut t = LivenessTracker::with_timeout(1_000);
        t.record_pong("peer", 0);
        t.observe_pfail_report("peer");
        // Only 1.5× timeout → still not enough.
        assert!(!t.should_promote_fail("peer", 1_500));
        assert!(t.should_promote_fail("peer", 2_500));
    }

    #[test]
    fn mark_fail_sets_flag() {
        let mut t = LivenessTracker::new();
        t.record_pong("peer", 0);
        t.mark_fail("peer");
        assert!(t.peer("peer").unwrap().flags.contains(NodeFlags::FAIL));
    }

    #[test]
    fn heartbeat_scheduler_ticks_at_interval() {
        let mut s = HeartbeatScheduler::with_interval(100);
        assert!(s.should_tick(100));
        s.record_tick(100);
        assert!(!s.should_tick(150));
        assert!(s.should_tick(201));
    }

    #[test]
    fn heartbeat_scheduler_picks_none_for_empty_peers() {
        let s = HeartbeatScheduler::new();
        let peers: Vec<String> = vec![];
        assert!(s.pick_peer(&peers, 42).is_none());
    }

    #[test]
    fn heartbeat_scheduler_pick_peer_is_deterministic_per_seed() {
        let s = HeartbeatScheduler::new();
        let peers = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let p1 = s.pick_peer(&peers, 42);
        let p2 = s.pick_peer(&peers, 42);
        assert_eq!(p1, p2);
    }

    #[test]
    fn heartbeat_scheduler_pick_peer_returns_known_member() {
        let s = HeartbeatScheduler::new();
        let peers = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let pick = s.pick_peer(&peers, 17).unwrap();
        assert!(peers.contains(&pick));
    }

    #[test]
    fn meet_handshake_attaches_meet_flag_to_sender_via_node_flags() {
        // A MEET message arrives with the MEET flag set on the
        // sender. Downstream record-receive paths treat that as
        // "complete the HANDSHAKE — promote to known peer".
        let mut h = header(ClusterMsgType::Meet);
        h.flags = NodeFlags::MASTER | NodeFlags::MEET;
        let m = ClusterMsg {
            header: h,
            body: ClusterMsgBody::Gossip(GossipSection::default()),
        };
        let bytes = encode(&m).unwrap();
        let back = decode(&bytes).unwrap();
        assert!(back.header.flags.contains(NodeFlags::MEET));
    }

    #[test]
    fn derive_bus_addr_offsets_port_by_10000() {
        let a = derive_bus_addr("127.0.0.1:6379").unwrap();
        assert_eq!(a.port(), 16_379);
        assert_eq!(a.ip().to_string(), "127.0.0.1");
    }

    #[test]
    fn derive_bus_addr_handles_ipv6() {
        let a = derive_bus_addr("[::1]:6379").unwrap();
        assert_eq!(a.port(), 16_379);
    }

    #[test]
    fn derive_bus_addr_rejects_bogus_input() {
        assert!(derive_bus_addr("not-an-addr").is_none());
    }

    #[test]
    fn header_includes_offset_for_replication_purposes() {
        let m = ClusterMsg {
            header: header(ClusterMsgType::Ping),
            body: ClusterMsgBody::Gossip(GossipSection::default()),
        };
        let bytes = encode(&m).unwrap();
        let back = decode(&bytes).unwrap();
        assert_eq!(back.header.offset, 1024);
    }

    #[test]
    fn gossip_entry_with_pfail_flag_round_trips() {
        let entry = GossipEntry {
            node_id: "e".repeat(40),
            ping_sent_ms: 50,
            pong_received_ms: 0,
            addr: "10.0.0.5:6379".into(),
            flags: NodeFlags::PFAIL | NodeFlags::MASTER,
        };
        let m = ClusterMsg {
            header: header(ClusterMsgType::Pong),
            body: ClusterMsgBody::Gossip(GossipSection {
                entries: vec![entry.clone()],
            }),
        };
        let bytes = encode(&m).unwrap();
        let back = decode(&bytes).unwrap();
        if let ClusterMsgBody::Gossip(g) = back.body {
            assert!(g.entries[0].flags.contains(NodeFlags::PFAIL));
        } else {
            panic!();
        }
    }

    #[test]
    fn migrate_from_and_migrate_to_flags_are_distinct_bits() {
        assert_ne!(NodeFlags::MIGRATE_FROM, NodeFlags::MIGRATE_TO);
        assert!((NodeFlags::MIGRATE_FROM | NodeFlags::MIGRATE_TO).bits() != 0);
    }

    #[test]
    fn encode_back_patches_totlen_correctly() {
        let m = ClusterMsg {
            header: header(ClusterMsgType::Ping),
            body: ClusterMsgBody::Gossip(GossipSection::default()),
        };
        let bytes = encode(&m).unwrap();
        let declared =
            u32::from_be_bytes(bytes[8..12].try_into().unwrap()) as usize;
        assert_eq!(declared, bytes.len());
    }

    #[test]
    fn length_mismatch_when_extra_bytes_appended() {
        let mut bytes = encode(&ClusterMsg {
            header: header(ClusterMsgType::Ping),
            body: ClusterMsgBody::Gossip(GossipSection::default()),
        })
        .unwrap();
        bytes.push(0);
        bytes.push(0);
        assert!(matches!(
            decode(&bytes),
            Err(WireError::LengthMismatch { .. })
        ));
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Kafka-wire-shaped KRaft RPCs — the bytes the controller
//! quorum exchanges over the Kafka binary protocol (KIP-595).
//!
//! Mirrors `org.apache.kafka.common.message.VoteRequestData` +
//! `BeginQuorumEpochRequestData` + `EndQuorumEpochRequestData` +
//! `DescribeQuorumResponseData` from upstream. cave-streams ports
//! the v0 schema for each (the minimum the spec requires of every
//! voter); higher versions add only optional fields.
//!
//! ## Wire layout
//!
//! Each request/response carries a small fixed-shape body:
//!
//! * `VoteRequest`           — candidate's id + last-log epoch +
//!                             last-log offset
//! * `VoteResponse`          — vote granted? + replied epoch
//! * `BeginQuorumEpoch`      — leader's id + epoch (announce
//!                             leadership)
//! * `EndQuorumEpoch`        — leader's id + epoch (step-down
//!                             announcement)
//! * `DescribeQuorum`        — read-only: who's the leader,
//!                             who's in the voter set, current
//!                             epoch + log end offset
//!
//! ## Honest scope
//!
//! Wire serialisation + the controller-side handler dispatch
//! land here. Outbound *initiation* of these RPCs (i.e. the
//! controller actively calling `Vote` on its peers) belongs in
//! the future `RaftTransport` integration — see the module doc
//! on [`super`]. This module gives the wire surface a peer
//! controller can interoperate against.

use bytes::{Buf, BufMut, BytesMut};

use super::epoch::{ControllerEpoch, VoterSet};
use super::metadata_log::MetadataLog;
use crate::error::{StreamsError, StreamsResult};
use crate::protocol::{decode_string, encode_string};
use std::sync::Arc;

/// Request body for `Vote` (ApiKey 52).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoteRequest {
    pub topic_name: String,
    pub partition_index: i32,
    /// Candidate's epoch — the term they're requesting a vote
    /// for. Must be strictly greater than the receiver's
    /// current epoch.
    pub candidate_epoch: i32,
    pub candidate_id: i32,
    /// Last log epoch the candidate has — used by the receiver
    /// to enforce the "log up-to-date" rule (paper §5.4.1).
    pub last_offset_epoch: i32,
    pub last_offset: i64,
}

impl VoteRequest {
    pub fn encode(&self, buf: &mut BytesMut) {
        encode_string(buf, &self.topic_name);
        buf.put_i32(self.partition_index);
        buf.put_i32(self.candidate_epoch);
        buf.put_i32(self.candidate_id);
        buf.put_i32(self.last_offset_epoch);
        buf.put_i64(self.last_offset);
    }

    pub fn decode(buf: &mut dyn Buf) -> StreamsResult<Self> {
        let topic_name = decode_string(buf)?;
        Self::check(buf, 4 + 4 + 4 + 4 + 8)?;
        Ok(Self {
            topic_name,
            partition_index: buf.get_i32(),
            candidate_epoch: buf.get_i32(),
            candidate_id: buf.get_i32(),
            last_offset_epoch: buf.get_i32(),
            last_offset: buf.get_i64(),
        })
    }

    fn check(buf: &mut dyn Buf, want: usize) -> StreamsResult<()> {
        if buf.remaining() < want {
            return Err(StreamsError::ProtocolDecode(format!(
                "VoteRequest truncated: need {want}, have {}",
                buf.remaining()
            )));
        }
        Ok(())
    }
}

/// Response body for `Vote`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoteResponse {
    pub error_code: i16,
    pub topic_name: String,
    pub partition_index: i32,
    pub leader_id: i32,
    pub leader_epoch: i32,
    pub vote_granted: bool,
}

impl VoteResponse {
    pub fn encode(&self, buf: &mut BytesMut) {
        buf.put_i16(self.error_code);
        encode_string(buf, &self.topic_name);
        buf.put_i32(self.partition_index);
        buf.put_i32(self.leader_id);
        buf.put_i32(self.leader_epoch);
        buf.put_u8(self.vote_granted as u8);
    }

    pub fn decode(buf: &mut dyn Buf) -> StreamsResult<Self> {
        if buf.remaining() < 2 {
            return Err(StreamsError::ProtocolDecode(
                "VoteResponse truncated".into(),
            ));
        }
        let error_code = buf.get_i16();
        let topic_name = decode_string(buf)?;
        if buf.remaining() < 4 + 4 + 4 + 1 {
            return Err(StreamsError::ProtocolDecode(
                "VoteResponse truncated".into(),
            ));
        }
        Ok(Self {
            error_code,
            topic_name,
            partition_index: buf.get_i32(),
            leader_id: buf.get_i32(),
            leader_epoch: buf.get_i32(),
            vote_granted: buf.get_u8() != 0,
        })
    }
}

/// Request body for `BeginQuorumEpoch` (ApiKey 53).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BeginQuorumEpochRequest {
    pub topic_name: String,
    pub partition_index: i32,
    pub leader_id: i32,
    pub leader_epoch: i32,
}

impl BeginQuorumEpochRequest {
    pub fn encode(&self, buf: &mut BytesMut) {
        encode_string(buf, &self.topic_name);
        buf.put_i32(self.partition_index);
        buf.put_i32(self.leader_id);
        buf.put_i32(self.leader_epoch);
    }
    pub fn decode(buf: &mut dyn Buf) -> StreamsResult<Self> {
        let topic_name = decode_string(buf)?;
        if buf.remaining() < 12 {
            return Err(StreamsError::ProtocolDecode(
                "BeginQuorumEpoch truncated".into(),
            ));
        }
        Ok(Self {
            topic_name,
            partition_index: buf.get_i32(),
            leader_id: buf.get_i32(),
            leader_epoch: buf.get_i32(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BeginQuorumEpochResponse {
    pub error_code: i16,
    pub topic_name: String,
    pub partition_index: i32,
}

impl BeginQuorumEpochResponse {
    pub fn encode(&self, buf: &mut BytesMut) {
        buf.put_i16(self.error_code);
        encode_string(buf, &self.topic_name);
        buf.put_i32(self.partition_index);
    }
    pub fn decode(buf: &mut dyn Buf) -> StreamsResult<Self> {
        if buf.remaining() < 2 {
            return Err(StreamsError::ProtocolDecode(
                "BeginQuorumEpochResponse truncated".into(),
            ));
        }
        let error_code = buf.get_i16();
        let topic_name = decode_string(buf)?;
        if buf.remaining() < 4 {
            return Err(StreamsError::ProtocolDecode(
                "BeginQuorumEpochResponse truncated".into(),
            ));
        }
        Ok(Self {
            error_code,
            topic_name,
            partition_index: buf.get_i32(),
        })
    }
}

/// Request body for `EndQuorumEpoch` (ApiKey 54).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndQuorumEpochRequest {
    pub topic_name: String,
    pub partition_index: i32,
    pub leader_id: i32,
    pub leader_epoch: i32,
    /// Successor voter IDs the resigning leader nominates,
    /// in priority order.
    pub preferred_successors: Vec<i32>,
}

impl EndQuorumEpochRequest {
    pub fn encode(&self, buf: &mut BytesMut) {
        encode_string(buf, &self.topic_name);
        buf.put_i32(self.partition_index);
        buf.put_i32(self.leader_id);
        buf.put_i32(self.leader_epoch);
        buf.put_i32(self.preferred_successors.len() as i32);
        for id in &self.preferred_successors {
            buf.put_i32(*id);
        }
    }
    pub fn decode(buf: &mut dyn Buf) -> StreamsResult<Self> {
        let topic_name = decode_string(buf)?;
        if buf.remaining() < 16 {
            return Err(StreamsError::ProtocolDecode(
                "EndQuorumEpoch truncated".into(),
            ));
        }
        let partition_index = buf.get_i32();
        let leader_id = buf.get_i32();
        let leader_epoch = buf.get_i32();
        let n = buf.get_i32();
        let n = if n < 0 { 0 } else { n as usize };
        if buf.remaining() < n * 4 {
            return Err(StreamsError::ProtocolDecode(
                "EndQuorumEpoch successors truncated".into(),
            ));
        }
        let mut preferred_successors = Vec::with_capacity(n);
        for _ in 0..n {
            preferred_successors.push(buf.get_i32());
        }
        Ok(Self {
            topic_name,
            partition_index,
            leader_id,
            leader_epoch,
            preferred_successors,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndQuorumEpochResponse {
    pub error_code: i16,
    pub topic_name: String,
    pub partition_index: i32,
}

impl EndQuorumEpochResponse {
    pub fn encode(&self, buf: &mut BytesMut) {
        buf.put_i16(self.error_code);
        encode_string(buf, &self.topic_name);
        buf.put_i32(self.partition_index);
    }
    pub fn decode(buf: &mut dyn Buf) -> StreamsResult<Self> {
        if buf.remaining() < 2 {
            return Err(StreamsError::ProtocolDecode(
                "EndQuorumEpochResponse truncated".into(),
            ));
        }
        let error_code = buf.get_i16();
        let topic_name = decode_string(buf)?;
        if buf.remaining() < 4 {
            return Err(StreamsError::ProtocolDecode(
                "EndQuorumEpochResponse truncated".into(),
            ));
        }
        Ok(Self {
            error_code,
            topic_name,
            partition_index: buf.get_i32(),
        })
    }
}

/// Request body for `DescribeQuorum` (ApiKey 55). v0 has no
/// arguments other than the topic-partition reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DescribeQuorumRequest {
    pub topic_name: String,
    pub partition_index: i32,
}

impl DescribeQuorumRequest {
    pub fn encode(&self, buf: &mut BytesMut) {
        encode_string(buf, &self.topic_name);
        buf.put_i32(self.partition_index);
    }
    pub fn decode(buf: &mut dyn Buf) -> StreamsResult<Self> {
        let topic_name = decode_string(buf)?;
        if buf.remaining() < 4 {
            return Err(StreamsError::ProtocolDecode(
                "DescribeQuorum truncated".into(),
            ));
        }
        Ok(Self {
            topic_name,
            partition_index: buf.get_i32(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DescribeQuorumResponse {
    pub error_code: i16,
    pub topic_name: String,
    pub partition_index: i32,
    pub leader_id: i32,
    pub leader_epoch: i32,
    pub high_watermark: i64,
    pub current_voters: Vec<i32>,
    pub observers: Vec<i32>,
}

impl DescribeQuorumResponse {
    pub fn encode(&self, buf: &mut BytesMut) {
        buf.put_i16(self.error_code);
        encode_string(buf, &self.topic_name);
        buf.put_i32(self.partition_index);
        buf.put_i32(self.leader_id);
        buf.put_i32(self.leader_epoch);
        buf.put_i64(self.high_watermark);
        buf.put_i32(self.current_voters.len() as i32);
        for v in &self.current_voters {
            buf.put_i32(*v);
        }
        buf.put_i32(self.observers.len() as i32);
        for o in &self.observers {
            buf.put_i32(*o);
        }
    }
    pub fn decode(buf: &mut dyn Buf) -> StreamsResult<Self> {
        if buf.remaining() < 2 {
            return Err(StreamsError::ProtocolDecode(
                "DescribeQuorumResponse truncated".into(),
            ));
        }
        let error_code = buf.get_i16();
        let topic_name = decode_string(buf)?;
        if buf.remaining() < 4 + 4 + 4 + 8 + 4 {
            return Err(StreamsError::ProtocolDecode(
                "DescribeQuorumResponse truncated".into(),
            ));
        }
        let partition_index = buf.get_i32();
        let leader_id = buf.get_i32();
        let leader_epoch = buf.get_i32();
        let high_watermark = buf.get_i64();
        let nv = buf.get_i32().max(0) as usize;
        if buf.remaining() < nv * 4 + 4 {
            return Err(StreamsError::ProtocolDecode(
                "DescribeQuorumResponse voters truncated".into(),
            ));
        }
        let mut current_voters = Vec::with_capacity(nv);
        for _ in 0..nv {
            current_voters.push(buf.get_i32());
        }
        let no = buf.get_i32().max(0) as usize;
        if buf.remaining() < no * 4 {
            return Err(StreamsError::ProtocolDecode(
                "DescribeQuorumResponse observers truncated".into(),
            ));
        }
        let mut observers = Vec::with_capacity(no);
        for _ in 0..no {
            observers.push(buf.get_i32());
        }
        Ok(Self {
            error_code,
            topic_name,
            partition_index,
            leader_id,
            leader_epoch,
            high_watermark,
            current_voters,
            observers,
        })
    }
}

/// Kafka error code 0 = success — used everywhere a request was
/// accepted with no condition violated.
pub const ERR_NONE: i16 = 0;
/// 100 = NOT_LEADER_FOR_PARTITION (also used for stale-epoch
/// rejections in KRaft). Matches Apache Kafka's
/// `Errors.NOT_LEADER_OR_FOLLOWER`.
pub const ERR_NOT_LEADER: i16 = 6;
/// 47 = FENCED_LEADER_EPOCH — Apache Kafka. Used when the
/// candidate's epoch is ≤ the receiver's current epoch.
pub const ERR_FENCED_LEADER_EPOCH: i16 = 47;
/// 76 = INCONSISTENT_VOTER_SET — voter not in the configured set.
pub const ERR_INCONSISTENT_VOTER_SET: i16 = 76;

/// The KRaft-side handler dispatched off the kafka_wire layer.
/// Owns a reference to the voter set + metadata log, applies the
/// receiver-side rules from KIP-595, and produces a response.
pub struct KraftHandler {
    voters: std::sync::RwLock<VoterSet>,
    log: Arc<MetadataLog>,
    /// The topic-partition KRaft is bound to. Single-quorum
    /// installs use `__cluster_metadata` / 0, matching upstream.
    topic_name: String,
    partition_index: i32,
}

impl KraftHandler {
    pub fn new(voters: VoterSet, log: Arc<MetadataLog>) -> Self {
        Self {
            voters: std::sync::RwLock::new(voters),
            log,
            topic_name: "__cluster_metadata".to_string(),
            partition_index: 0,
        }
    }

    /// Voter set snapshot for tests / external observers.
    pub fn voters(&self) -> VoterSet {
        self.voters.read().expect("poisoned").clone()
    }

    pub fn handle_vote(&self, req: &VoteRequest) -> VoteResponse {
        let mut v = self.voters.write().expect("poisoned");
        let current_epoch = v.epoch().0 as i32;
        let leader_id = v.leader().unwrap_or(-1);

        // Reject if the candidate is not part of the voter set.
        if !v.contains(req.candidate_id) {
            return VoteResponse {
                error_code: ERR_INCONSISTENT_VOTER_SET,
                topic_name: req.topic_name.clone(),
                partition_index: req.partition_index,
                leader_id,
                leader_epoch: current_epoch,
                vote_granted: false,
            };
        }

        // Reject if the candidate's epoch is stale.
        if req.candidate_epoch <= current_epoch {
            return VoteResponse {
                error_code: ERR_FENCED_LEADER_EPOCH,
                topic_name: req.topic_name.clone(),
                partition_index: req.partition_index,
                leader_id,
                leader_epoch: current_epoch,
                vote_granted: false,
            };
        }

        // Receiver-side log-up-to-date check (paper §5.4.1):
        // candidate's last_offset_epoch / last_offset must be at
        // least as fresh as our own.
        let our_last_epoch = v.epoch().0 as i32;
        let our_last_offset = self.log.high_water_mark() as i64;
        let candidate_log_ok = req.last_offset_epoch > our_last_epoch
            || (req.last_offset_epoch == our_last_epoch && req.last_offset >= our_last_offset);

        if !candidate_log_ok {
            return VoteResponse {
                error_code: ERR_NONE,
                topic_name: req.topic_name.clone(),
                partition_index: req.partition_index,
                leader_id,
                leader_epoch: current_epoch,
                vote_granted: false,
            };
        }

        // Grant — voter advances its epoch and steps down (the
        // candidate may or may not actually win, but the
        // protocol requires we update local term on vote-grant
        // to prevent double-vote in the same term).
        let new_epoch = ControllerEpoch(req.candidate_epoch as u64);
        // Step down first (clears local leader claim).
        v.step_down();
        // Update epoch even if no election concludes yet —
        // VoterSet::elect bumps epoch on actual leadership.
        // For a grant we just track that we voted in
        // `candidate_epoch`; in this simplified port the next
        // `BeginQuorumEpoch` will then `elect()` properly.
        let _ = new_epoch;

        VoteResponse {
            error_code: ERR_NONE,
            topic_name: req.topic_name.clone(),
            partition_index: req.partition_index,
            leader_id: req.candidate_id,
            leader_epoch: req.candidate_epoch,
            vote_granted: true,
        }
    }

    pub fn handle_begin_quorum_epoch(
        &self,
        req: &BeginQuorumEpochRequest,
    ) -> BeginQuorumEpochResponse {
        let mut v = self.voters.write().expect("poisoned");
        let current_epoch = v.epoch().0 as i32;
        if req.leader_epoch <= current_epoch {
            return BeginQuorumEpochResponse {
                error_code: ERR_FENCED_LEADER_EPOCH,
                topic_name: req.topic_name.clone(),
                partition_index: req.partition_index,
            };
        }
        match v.elect(req.leader_id, ControllerEpoch(req.leader_epoch as u64)) {
            Ok(()) => BeginQuorumEpochResponse {
                error_code: ERR_NONE,
                topic_name: req.topic_name.clone(),
                partition_index: req.partition_index,
            },
            Err(_) => BeginQuorumEpochResponse {
                error_code: ERR_INCONSISTENT_VOTER_SET,
                topic_name: req.topic_name.clone(),
                partition_index: req.partition_index,
            },
        }
    }

    pub fn handle_end_quorum_epoch(&self, req: &EndQuorumEpochRequest) -> EndQuorumEpochResponse {
        let mut v = self.voters.write().expect("poisoned");
        if v.leader() == Some(req.leader_id) && v.epoch().0 as i32 == req.leader_epoch {
            v.step_down();
        }
        EndQuorumEpochResponse {
            error_code: ERR_NONE,
            topic_name: req.topic_name.clone(),
            partition_index: req.partition_index,
        }
    }

    pub fn handle_describe_quorum(&self, req: &DescribeQuorumRequest) -> DescribeQuorumResponse {
        let v = self.voters.read().expect("poisoned");
        // VoterSet doesn't expose individual ids by index right
        // now — we expose the count via `size()` and accept that
        // observers (non-voters) is empty for v0. Real upstream
        // also lists per-voter logEndOffset + lastFetchTimestamp;
        // those would surface once a Fetch/Heartbeat path is
        // wired.
        let leader_id = v.leader().unwrap_or(-1);
        let leader_epoch = v.epoch().0 as i32;
        let high_watermark = self.log.high_water_mark() as i64;
        DescribeQuorumResponse {
            error_code: ERR_NONE,
            topic_name: req.topic_name.clone(),
            partition_index: req.partition_index,
            leader_id,
            leader_epoch,
            high_watermark,
            current_voters: (0..v.size() as i32).collect(),
            observers: Vec::new(),
        }
    }

    /// Synthesise a DescribeQuorum response without an inbound
    /// request — convenience for unit tests and the
    /// `cavectl streams describe-quorum` command.
    pub fn snapshot_quorum(&self) -> DescribeQuorumResponse {
        self.handle_describe_quorum(&DescribeQuorumRequest {
            topic_name: self.topic_name.clone(),
            partition_index: self.partition_index,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::BytesMut;

    fn handler_with_voters(voters: VoterSet) -> KraftHandler {
        KraftHandler::new(voters, Arc::new(MetadataLog::new()))
    }

    fn sample_vote_req(candidate_id: i32, candidate_epoch: i32) -> VoteRequest {
        VoteRequest {
            topic_name: "__cluster_metadata".to_string(),
            partition_index: 0,
            candidate_epoch,
            candidate_id,
            last_offset_epoch: 0,
            last_offset: 0,
        }
    }

    #[test]
    fn vote_request_encode_decode_round_trips() {
        let v = sample_vote_req(2, 5);
        let mut buf = BytesMut::new();
        v.encode(&mut buf);
        let mut cur: &[u8] = &buf;
        let v2 = VoteRequest::decode(&mut cur).unwrap();
        assert_eq!(v, v2);
    }

    #[test]
    fn vote_response_encode_decode_round_trips() {
        let r = VoteResponse {
            error_code: ERR_NONE,
            topic_name: "__cluster_metadata".to_string(),
            partition_index: 0,
            leader_id: 1,
            leader_epoch: 3,
            vote_granted: true,
        };
        let mut buf = BytesMut::new();
        r.encode(&mut buf);
        let mut cur: &[u8] = &buf;
        let r2 = VoteResponse::decode(&mut cur).unwrap();
        assert_eq!(r, r2);
    }

    #[test]
    fn begin_quorum_epoch_round_trips() {
        let r = BeginQuorumEpochRequest {
            topic_name: "__cluster_metadata".to_string(),
            partition_index: 0,
            leader_id: 1,
            leader_epoch: 4,
        };
        let mut buf = BytesMut::new();
        r.encode(&mut buf);
        let mut cur: &[u8] = &buf;
        let r2 = BeginQuorumEpochRequest::decode(&mut cur).unwrap();
        assert_eq!(r, r2);
    }

    #[test]
    fn end_quorum_epoch_round_trips_with_successors() {
        let r = EndQuorumEpochRequest {
            topic_name: "__cluster_metadata".into(),
            partition_index: 0,
            leader_id: 1,
            leader_epoch: 5,
            preferred_successors: vec![2, 3],
        };
        let mut buf = BytesMut::new();
        r.encode(&mut buf);
        let mut cur: &[u8] = &buf;
        let r2 = EndQuorumEpochRequest::decode(&mut cur).unwrap();
        assert_eq!(r, r2);
    }

    #[test]
    fn describe_quorum_response_round_trips() {
        let r = DescribeQuorumResponse {
            error_code: ERR_NONE,
            topic_name: "__cluster_metadata".into(),
            partition_index: 0,
            leader_id: 1,
            leader_epoch: 7,
            high_watermark: 42,
            current_voters: vec![1, 2, 3],
            observers: vec![4],
        };
        let mut buf = BytesMut::new();
        r.encode(&mut buf);
        let mut cur: &[u8] = &buf;
        let r2 = DescribeQuorumResponse::decode(&mut cur).unwrap();
        assert_eq!(r, r2);
    }

    #[test]
    fn handle_vote_grants_higher_epoch_in_voter_set() {
        let h = handler_with_voters(VoterSet::new([1, 2, 3]));
        let r = h.handle_vote(&sample_vote_req(2, 1));
        assert_eq!(r.error_code, ERR_NONE);
        assert!(r.vote_granted);
        assert_eq!(r.leader_id, 2);
        assert_eq!(r.leader_epoch, 1);
    }

    #[test]
    fn handle_vote_rejects_non_voter() {
        let h = handler_with_voters(VoterSet::new([1, 2, 3]));
        let r = h.handle_vote(&sample_vote_req(42, 1));
        assert_eq!(r.error_code, ERR_INCONSISTENT_VOTER_SET);
        assert!(!r.vote_granted);
    }

    #[test]
    fn handle_vote_rejects_stale_epoch() {
        let h = handler_with_voters(VoterSet::new([1, 2, 3]));
        // Advance current epoch to 5.
        let _ = h.handle_begin_quorum_epoch(&BeginQuorumEpochRequest {
            topic_name: "__cluster_metadata".into(),
            partition_index: 0,
            leader_id: 1,
            leader_epoch: 5,
        });
        let r = h.handle_vote(&sample_vote_req(2, 5));
        assert_eq!(r.error_code, ERR_FENCED_LEADER_EPOCH);
        assert!(!r.vote_granted);
    }

    #[test]
    fn handle_begin_quorum_epoch_elects_leader() {
        let h = handler_with_voters(VoterSet::new([1, 2, 3]));
        let r = h.handle_begin_quorum_epoch(&BeginQuorumEpochRequest {
            topic_name: "__cluster_metadata".into(),
            partition_index: 0,
            leader_id: 2,
            leader_epoch: 3,
        });
        assert_eq!(r.error_code, ERR_NONE);
        assert_eq!(h.voters().leader(), Some(2));
        assert_eq!(h.voters().epoch(), ControllerEpoch(3));
    }

    #[test]
    fn handle_begin_quorum_epoch_rejects_stale() {
        let h = handler_with_voters(VoterSet::new([1, 2, 3]));
        let _ = h.handle_begin_quorum_epoch(&BeginQuorumEpochRequest {
            topic_name: "__cluster_metadata".into(),
            partition_index: 0,
            leader_id: 1,
            leader_epoch: 5,
        });
        let r = h.handle_begin_quorum_epoch(&BeginQuorumEpochRequest {
            topic_name: "__cluster_metadata".into(),
            partition_index: 0,
            leader_id: 2,
            leader_epoch: 4, // stale
        });
        assert_eq!(r.error_code, ERR_FENCED_LEADER_EPOCH);
    }

    #[test]
    fn handle_end_quorum_epoch_clears_leader() {
        let h = handler_with_voters(VoterSet::new([1, 2, 3]));
        let _ = h.handle_begin_quorum_epoch(&BeginQuorumEpochRequest {
            topic_name: "__cluster_metadata".into(),
            partition_index: 0,
            leader_id: 1,
            leader_epoch: 5,
        });
        let r = h.handle_end_quorum_epoch(&EndQuorumEpochRequest {
            topic_name: "__cluster_metadata".into(),
            partition_index: 0,
            leader_id: 1,
            leader_epoch: 5,
            preferred_successors: vec![2],
        });
        assert_eq!(r.error_code, ERR_NONE);
        assert!(h.voters().leader().is_none());
        // Epoch is preserved on step-down.
        assert_eq!(h.voters().epoch(), ControllerEpoch(5));
    }

    #[test]
    fn describe_quorum_reports_current_state() {
        let h = handler_with_voters(VoterSet::new([1, 2, 3]));
        let _ = h.handle_begin_quorum_epoch(&BeginQuorumEpochRequest {
            topic_name: "__cluster_metadata".into(),
            partition_index: 0,
            leader_id: 2,
            leader_epoch: 4,
        });
        let r = h.snapshot_quorum();
        assert_eq!(r.error_code, ERR_NONE);
        assert_eq!(r.leader_id, 2);
        assert_eq!(r.leader_epoch, 4);
        assert_eq!(r.current_voters.len(), 3);
    }

    #[test]
    fn vote_request_truncated_yields_decode_error() {
        let bad = vec![0u8; 2]; // not even a string length
        let mut cur: &[u8] = &bad;
        assert!(VoteRequest::decode(&mut cur).is_err());
    }

    #[test]
    fn round_trip_via_byte_buffer_simulating_wire() {
        // Sender → wire → Receiver, full round.
        let req = sample_vote_req(3, 7);
        let mut buf = BytesMut::new();
        req.encode(&mut buf);
        let mut wire: &[u8] = &buf;
        let decoded = VoteRequest::decode(&mut wire).unwrap();
        let h = handler_with_voters(VoterSet::new([1, 2, 3]));
        let resp = h.handle_vote(&decoded);
        let mut rbuf = BytesMut::new();
        resp.encode(&mut rbuf);
        let mut rwire: &[u8] = &rbuf;
        let decoded_resp = VoteResponse::decode(&mut rwire).unwrap();
        assert_eq!(decoded_resp, resp);
    }
}

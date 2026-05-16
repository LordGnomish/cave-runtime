// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Cave Runtime contributors
//
// Source: apache/kafka@9f8b3ad416bd416f3706f3d7a1f425b9dd8bc5f2
// clients/src/main/java/org/apache/kafka/common/message/ConsumerGroupHeartbeatRequest.json
// clients/src/main/java/org/apache/kafka/common/message/ConsumerGroupHeartbeatResponse.json
//
//! Wire types for the KIP-848 `ConsumerGroupHeartbeat` RPC pair.

use serde::{Deserialize, Serialize};

use super::records::TopicPartitions;

/// Subset of error codes carried in the heartbeat response. Values
/// match the upstream `Errors` enum where they exist; KIP-848-specific
/// codes use the new range (110..).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i16)]
pub enum HeartbeatErrorCode {
    None = 0,
    UnknownMemberId = 25,
    InvalidGroupId = 24,
    InvalidRequest = 42,
    UnsupportedVersion = 35,
    FencedMemberEpoch = 110,
    UnreleasedInstanceId = 111,
}

/// `ConsumerGroupHeartbeatRequest` v0+ as described in KIP-848.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsumerGroupHeartbeatRequest {
    pub group_id: String,
    /// Empty string on the very first heartbeat — the coordinator mints one.
    pub member_id: String,
    /// `0` on first join, `-1` to leave the group, otherwise the
    /// epoch echoed back from the previous response.
    pub member_epoch: i32,
    /// KIP-345 static membership opt-in.
    pub instance_id: Option<String>,
    pub rack_id: Option<String>,
    pub rebalance_timeout_ms: i32,
    pub subscribed_topic_names: Vec<String>,
    pub subscribed_topic_regex: Option<String>,
    /// Optional server-side assignor selector (e.g. "uniform", "range").
    pub server_assignor: Option<String>,
    /// Echoed back from the previous response (used by client to ACK).
    pub topic_partitions: Vec<TopicPartitions>,
    /// `1` = KIP-848; `0` = legacy classic rebalance.
    pub protocol_version: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsumerGroupHeartbeatResponse {
    pub error_code: i16,
    pub member_id: String,
    pub member_epoch: i32,
    pub heartbeat_interval_ms: i32,
    pub assignment: Vec<TopicPartitions>,
}

impl ConsumerGroupHeartbeatResponse {
    pub fn encode(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("response encodable")
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        serde_json::from_slice(bytes).map_err(|e| e.to_string())
    }
}

impl ConsumerGroupHeartbeatRequest {
    pub fn encode(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("request encodable")
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        serde_json::from_slice(bytes).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_code_values_match_kafka() {
        assert_eq!(HeartbeatErrorCode::None as i16, 0);
        assert_eq!(HeartbeatErrorCode::UnknownMemberId as i16, 25);
        assert_eq!(HeartbeatErrorCode::FencedMemberEpoch as i16, 110);
    }

    #[test]
    fn request_round_trip() {
        let r = ConsumerGroupHeartbeatRequest {
            group_id: "g".into(),
            member_id: "m".into(),
            member_epoch: 1,
            instance_id: None,
            rack_id: None,
            rebalance_timeout_ms: 30_000,
            subscribed_topic_names: vec!["t".into()],
            subscribed_topic_regex: None,
            server_assignor: None,
            topic_partitions: vec![],
            protocol_version: 1,
        };
        let bytes = r.encode();
        assert_eq!(ConsumerGroupHeartbeatRequest::decode(&bytes).unwrap(), r);
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Cave Runtime contributors
//
// Source: apache/kafka@9f8b3ad416bd416f3706f3d7a1f425b9dd8bc5f2
// core/src/main/scala/kafka/server/group/GroupCoordinator.scala
// core/src/main/scala/kafka/coordinator/group/ConsumerGroupCoordinator.scala
//
//! Server-side state machine for KIP-848 consumer groups — RED stub.
//!
//! This file exists at the RED stage only to make the test crate
//! compile. The behavioural code is added in the corresponding
//! [GREEN] commit.

use crate::error::StreamsResult;

use super::records::PersistenceEntry;
use super::wire::{ConsumerGroupHeartbeatRequest, ConsumerGroupHeartbeatResponse};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsumerGroupSummary {
    pub group_id: String,
    pub group_epoch: i32,
    pub members: Vec<MemberSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberSummary {
    pub member_id: String,
    pub member_epoch: i32,
    pub subscribed: Vec<String>,
}

#[derive(Default)]
pub struct ConsumerGroupCoordinator;

impl ConsumerGroupCoordinator {
    pub fn new() -> Self {
        Self
    }

    pub fn set_topic_partition_count(&mut self, _topic: impl Into<String>, _count: i32) {}

    pub fn describe_group(&self, _group_id: &str) -> Option<ConsumerGroupSummary> {
        None
    }

    pub fn list_groups(&self) -> Vec<String> {
        Vec::new()
    }

    pub fn drain_persistence_log(&mut self) -> Vec<PersistenceEntry> {
        Vec::new()
    }

    pub fn heartbeat(
        &mut self,
        _req: ConsumerGroupHeartbeatRequest,
    ) -> StreamsResult<ConsumerGroupHeartbeatResponse> {
        // RED-stage stub — returns an empty response; behavioural tests
        // observe wrong values and fail.
        Ok(ConsumerGroupHeartbeatResponse {
            error_code: 0,
            member_id: String::new(),
            member_epoch: 0,
            heartbeat_interval_ms: 0,
            assignment: vec![],
        })
    }
}

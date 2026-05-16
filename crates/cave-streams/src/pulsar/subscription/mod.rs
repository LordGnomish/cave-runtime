// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Cave Runtime contributors
// Source: apache/pulsar@1940aebc6ade10050399cd65f870353eedf80008
//   pulsar-broker/.../service/persistent/PersistentSubscription.java
//   pulsar-client-api/src/main/java/org/apache/pulsar/client/api/SubscriptionType.java
//   pulsar-broker/.../service/persistent/PersistentDispatcherSingleActiveConsumer.java
//   pulsar-broker/.../service/persistent/PersistentDispatcherMultipleConsumers.java
//   pulsar-broker/.../service/persistent/PersistentStickyKeyDispatcherMultipleConsumers.java

//! Subscription-type dispatch policies (Exclusive / Shared / Failover /
//! Key_Shared) as a trait-shaped layer.
//!
//! The legacy [`crate::pulsar_dispatch`] module already owns a working
//! dispatcher that returns a consumer-id per message; this module
//! generalises it into a [`SubscriptionPolicy`] trait so the
//! Functions runtime and the replicator can plug in their own custom
//! consumer-set fan-out without re-implementing routing.
//!
//! Key_Shared variants:
//! - [`KeySharedMode::AutoSplit`] — Pulsar's default; equal hash slices
//!   per consumer (`HashRangeAutoSplitStickyKeyConsumerSelector`).
//! - [`KeySharedMode::Sticky`] — explicit hash ranges per consumer
//!   (`HashRangeExclusiveStickyKeyConsumerSelector`); each consumer
//!   declares `[low, high]` slots inside the 16-bit hash space.

pub mod exclusive;
pub mod failover;
pub mod key_shared;
pub mod shared;

use crate::error::StreamsResult;
pub use exclusive::ExclusivePolicy;
pub use failover::FailoverPolicy;
pub use key_shared::{KeySharedMode, KeySharedPolicy, StickyRange};
pub use shared::SharedPolicy;

/// Per-consumer state any policy can read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyConsumer {
    pub consumer_id: u64,
    pub priority: i32,
    pub has_permits: bool,
}

impl PolicyConsumer {
    pub fn new(consumer_id: u64) -> Self {
        Self {
            consumer_id,
            priority: 0,
            has_permits: true,
        }
    }
}

/// Trait every subscription type satisfies.  The dispatcher feeds it
/// the message (key) and the current consumer set; the policy picks a
/// recipient.
pub trait SubscriptionPolicy: Send + Sync {
    /// Add a consumer; the policy may reject (e.g. Exclusive when a
    /// consumer is already present).
    fn add_consumer(&mut self, c: PolicyConsumer) -> StreamsResult<()>;

    /// Remove the consumer.  Silent no-op when not present.
    fn remove_consumer(&mut self, consumer_id: u64);

    /// Pick a recipient for the next message.  `key` is consulted only
    /// by Key_Shared.
    fn pick(&mut self, key: Option<&[u8]>) -> Option<u64>;

    fn consumer_count(&self) -> usize;
}

/// Hash a key to a 16-bit slot — Pulsar uses 65536-bucket Murmur3.
/// cave-streams uses FNV-1a 16 (stable and dependency-free); a real
/// client would need Murmur3 for wire compatibility.
pub fn key_to_slot(key: &[u8]) -> u16 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in key {
        h ^= b as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    (h as u16) & 0xFFFF
}

/// Maximum hash-range slot (Pulsar's `KeySharedPolicy.DEFAULT_HASH_RANGE_SIZE` = 65536).
pub const HASH_RANGE_SIZE: u32 = 65_536;

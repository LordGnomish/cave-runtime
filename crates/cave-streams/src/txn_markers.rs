// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Kafka EOS transaction markers — `WriteTxnMarkersRequest` / Response
//! shape, plus a per-broker marker store that records committed/aborted
//! transaction outcomes per-partition.
//!
//! Mirrors the Kafka 4.2.0
//! `clients/src/main/java/org/apache/kafka/common/message/WriteTxnMarkersRequest.json`
//! schema.  Used by [`crate::transactions::TransactionCoordinator`] when
//! a transaction completes — the coordinator broadcasts a marker to every
//! enrolled partition so consumers can decide which records are part of a
//! committed transaction.

use crate::error::{StreamsError, StreamsResult};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicI64, Ordering};

/// `TransactionResult` enum from `WriteTxnMarkersRequest.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TxnMarkerResult {
    /// `COMMIT` — committed transaction.
    Commit,
    /// `ABORT` — rolled-back transaction.
    Abort,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TxnMarker {
    pub producer_id: i64,
    pub producer_epoch: i16,
    pub coordinator_epoch: i32,
    pub result: TxnMarkerResult,
    /// Last stable offset (LSO) at which the marker was appended on this
    /// partition.  Consumers in `read_committed` mode skip records whose
    /// offset is between the begin marker and this LSO if the result is
    /// `Abort`.
    pub last_stable_offset: i64,
}

/// Request shape — mirrors `WriteTxnMarkersRequest`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WriteTxnMarkersRequest {
    pub markers: Vec<TxnMarkerEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TxnMarkerEntry {
    pub producer_id: i64,
    pub producer_epoch: i16,
    pub coordinator_epoch: i32,
    pub result: TxnMarkerResult,
    pub partitions: Vec<(String, i32)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WriteTxnMarkersResponse {
    pub markers: Vec<TxnMarkerResponseEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TxnMarkerResponseEntry {
    pub producer_id: i64,
    /// `(topic, partition) → kafka error code` (0 = no error).
    pub topics: Vec<(String, i32, i16)>,
}

/// Per-partition log position assigned to each appended marker.
type PartitionKey = (String, i32);

/// Broker-side marker store — append-only by `(producer_id,
/// coordinator_epoch, partition)`.  The leader for a partition is the
/// only writer; followers replicate via the regular ISR path so a
/// single-broker registry is correct for in-process tests.
pub struct TxnMarkerStore {
    /// Per-partition next-offset (sub for the real log).
    next_offset: DashMap<PartitionKey, AtomicI64>,
    /// Per-partition append-only marker log.
    markers: DashMap<PartitionKey, Vec<TxnMarker>>,
    /// `producer_id → last successful coordinator_epoch` — used to fence
    /// stale markers per KIP-360.
    last_epoch: DashMap<i64, i32>,
}

impl TxnMarkerStore {
    pub fn new() -> Self {
        Self {
            next_offset: DashMap::new(),
            markers: DashMap::new(),
            last_epoch: DashMap::new(),
        }
    }

    /// Append `marker` to the named partition; returns the assigned LSO
    /// of the appended marker.  Rejects markers from a *stale* coordinator
    /// epoch with `Internal("STALE_COORDINATOR_EPOCH")`.
    pub fn append_marker(
        &self,
        topic: &str,
        partition: i32,
        marker: TxnMarker,
    ) -> StreamsResult<i64> {
        if let Some(prev) = self.last_epoch.get(&marker.producer_id) {
            if marker.coordinator_epoch < *prev {
                return Err(StreamsError::Internal(format!(
                    "STALE_COORDINATOR_EPOCH: pid={}, got={}, last={}",
                    marker.producer_id, marker.coordinator_epoch, *prev
                )));
            }
        }
        self.last_epoch
            .insert(marker.producer_id, marker.coordinator_epoch);
        let key = (topic.to_string(), partition);
        let off_atom = self
            .next_offset
            .entry(key.clone())
            .or_insert_with(|| AtomicI64::new(0));
        let assigned = off_atom.fetch_add(1, Ordering::SeqCst);
        let mut written = TxnMarker {
            last_stable_offset: assigned,
            ..marker
        };
        written.last_stable_offset = assigned;
        self.markers.entry(key).or_default().push(written);
        Ok(assigned)
    }

    /// Apply a full `WriteTxnMarkersRequest` and synthesise the response.
    pub fn apply(&self, req: &WriteTxnMarkersRequest) -> WriteTxnMarkersResponse {
        let mut out = Vec::with_capacity(req.markers.len());
        for entry in &req.markers {
            let mut topics = Vec::with_capacity(entry.partitions.len());
            for (topic, part) in &entry.partitions {
                let m = TxnMarker {
                    producer_id: entry.producer_id,
                    producer_epoch: entry.producer_epoch,
                    coordinator_epoch: entry.coordinator_epoch,
                    result: entry.result,
                    last_stable_offset: 0,
                };
                let err = self
                    .append_marker(topic, *part, m)
                    .map(|_| 0i16)
                    .unwrap_or_else(|_| 49 /* CONCURRENT_TRANSACTIONS */);
                topics.push((topic.clone(), *part, err));
            }
            out.push(TxnMarkerResponseEntry {
                producer_id: entry.producer_id,
                topics,
            });
        }
        WriteTxnMarkersResponse { markers: out }
    }

    pub fn list_markers(&self, topic: &str, partition: i32) -> Vec<TxnMarker> {
        self.markers
            .get(&(topic.to_string(), partition))
            .map(|r| r.clone())
            .unwrap_or_default()
    }

    /// Compute the LSO (last stable offset) for a partition: the maximum
    /// `last_stable_offset` among all appended markers, or 0 if none.
    pub fn last_stable_offset(&self, topic: &str, partition: i32) -> i64 {
        self.markers
            .get(&(topic.to_string(), partition))
            .and_then(|m| m.iter().map(|x| x.last_stable_offset).max())
            .unwrap_or(0)
    }

    pub fn last_epoch(&self, producer_id: i64) -> Option<i32> {
        self.last_epoch.get(&producer_id).map(|e| *e)
    }
}

impl Default for TxnMarkerStore {
    fn default() -> Self {
        Self::new()
    }
}

// ─────────────────────────────────────────────────────────────────────────
// EOS txn-marker tests — feat/cave-streams-deeper-001
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn topic(tenant_id: &str, suffix: &str) -> String {
        format!("tenants/{}/{}", tenant_id, suffix)
    }

    #[test]
    fn test_txn_marker_append_assigns_lso() {
        // cite: kafka 4.2.0 core/.../coordinator/transaction/TransactionStateManager
        let tenant_id = "txm-001";
        let store = TxnMarkerStore::new();
        let off = store
            .append_marker(
                &topic(tenant_id, "t"),
                0,
                TxnMarker {
                    producer_id: 100,
                    producer_epoch: 1,
                    coordinator_epoch: 1,
                    result: TxnMarkerResult::Commit,
                    last_stable_offset: 0,
                },
            )
            .unwrap();
        assert_eq!(off, 0);
        let off2 = store
            .append_marker(
                &topic(tenant_id, "t"),
                0,
                TxnMarker {
                    producer_id: 101,
                    producer_epoch: 1,
                    coordinator_epoch: 1,
                    result: TxnMarkerResult::Abort,
                    last_stable_offset: 0,
                },
            )
            .unwrap();
        assert_eq!(off2, 1);
    }

    #[test]
    fn test_txn_marker_apply_request_returns_response() {
        // cite: kafka 4.2.0 message/WriteTxnMarkersRequest.json + Response.json
        let tenant_id = "txm-002";
        let store = TxnMarkerStore::new();
        let req = WriteTxnMarkersRequest {
            markers: vec![TxnMarkerEntry {
                producer_id: 7,
                producer_epoch: 0,
                coordinator_epoch: 1,
                result: TxnMarkerResult::Commit,
                partitions: vec![
                    (topic(tenant_id, "a"), 0),
                    (topic(tenant_id, "b"), 0),
                ],
            }],
        };
        let resp = store.apply(&req);
        assert_eq!(resp.markers.len(), 1);
        assert_eq!(resp.markers[0].topics.len(), 2);
        assert!(resp.markers[0].topics.iter().all(|(_, _, e)| *e == 0));
    }

    #[test]
    fn test_txn_marker_stale_coordinator_rejected() {
        // cite: kafka 4.2.0 errors.STALE_COORDINATOR_EPOCH
        let tenant_id = "txm-003";
        let store = TxnMarkerStore::new();
        store
            .append_marker(
                &topic(tenant_id, "t"),
                0,
                TxnMarker {
                    producer_id: 1,
                    producer_epoch: 0,
                    coordinator_epoch: 5,
                    result: TxnMarkerResult::Commit,
                    last_stable_offset: 0,
                },
            )
            .unwrap();
        let err = store.append_marker(
            &topic(tenant_id, "t"),
            0,
            TxnMarker {
                producer_id: 1,
                producer_epoch: 0,
                coordinator_epoch: 3, // stale
                result: TxnMarkerResult::Commit,
                last_stable_offset: 0,
            },
        );
        assert!(matches!(err, Err(StreamsError::Internal(_))));
    }

    #[test]
    fn test_txn_marker_lso_is_max_per_partition() {
        // cite: kafka 4.2.0 LogStartOffset / LSO calculation
        let tenant_id = "txm-004";
        let store = TxnMarkerStore::new();
        for i in 0..3 {
            store
                .append_marker(
                    &topic(tenant_id, "t"),
                    0,
                    TxnMarker {
                        producer_id: 1,
                        producer_epoch: 0,
                        coordinator_epoch: 1,
                        result: TxnMarkerResult::Commit,
                        last_stable_offset: i as i64, // overwritten by store
                    },
                )
                .unwrap();
        }
        assert_eq!(store.last_stable_offset(&topic(tenant_id, "t"), 0), 2);
    }

    #[test]
    fn test_txn_marker_list_returns_in_order() {
        // cite: kafka 4.2.0 (per-partition append order)
        let tenant_id = "txm-005";
        let store = TxnMarkerStore::new();
        for pid in [10i64, 20, 30] {
            store
                .append_marker(
                    &topic(tenant_id, "t"),
                    0,
                    TxnMarker {
                        producer_id: pid,
                        producer_epoch: 0,
                        coordinator_epoch: 1,
                        result: TxnMarkerResult::Commit,
                        last_stable_offset: 0,
                    },
                )
                .unwrap();
        }
        let list = store.list_markers(&topic(tenant_id, "t"), 0);
        assert_eq!(list.iter().map(|m| m.producer_id).collect::<Vec<_>>(), vec![10, 20, 30]);
    }

    #[test]
    fn test_txn_marker_last_epoch_tracks_latest() {
        // cite: kafka 4.2.0 ProducerStateManager last-epoch tracking
        let tenant_id = "txm-006";
        let store = TxnMarkerStore::new();
        store
            .append_marker(
                &topic(tenant_id, "t"),
                0,
                TxnMarker {
                    producer_id: 1,
                    producer_epoch: 0,
                    coordinator_epoch: 4,
                    result: TxnMarkerResult::Commit,
                    last_stable_offset: 0,
                },
            )
            .unwrap();
        assert_eq!(store.last_epoch(1), Some(4));
    }
}

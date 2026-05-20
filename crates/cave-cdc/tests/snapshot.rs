// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! cave-cdc — snapshot + incremental snapshot tests.
//! Pinned to debezium-connector-common
//! `pipeline/source/spi/SnapshotChangeEventSource.java` +
//! `IncrementalSnapshotContext`.

use cave_cdc::CdcError;
use cave_cdc::snapshot::{SnapshotMode, SnapshotProgress};

const TENANT: &str = "tenant-acme-prod";

/// Cite: debezium docs `snapshot.mode` enum — the six legal values.
#[test]
fn snapshot_mode_parses_canonical_six_values() {
    use SnapshotMode::*;
    let cases = [
        ("initial", Initial),
        ("initial_only", InitialOnly),
        ("never", Never),
        ("when_needed", WhenNeeded),
        ("schema_only", SchemaOnly),
        ("schema_only_recovery", SchemaOnlyRecovery),
    ];
    for (s, mode) in cases {
        assert_eq!(SnapshotMode::parse(s).unwrap(), mode);
    }
    assert!(SnapshotMode::parse("garbage").is_err());
    let _ = TENANT;
}

/// Cite: debezium docs — `initial` / `initial_only` / `when_needed`
/// emit data records during the snapshot phase; `never` /
/// `schema_only*` do not. `initial_only` does NOT stream after the
/// snapshot completes.
#[test]
fn snapshot_mode_predicates_match_documented_behaviour() {
    use SnapshotMode::*;
    for m in [Initial, InitialOnly, WhenNeeded] {
        assert!(m.captures_data(), "{:?} captures data", m);
    }
    for m in [Never, SchemaOnly, SchemaOnlyRecovery] {
        assert!(!m.captures_data(), "{:?} does NOT capture data", m);
    }
    assert!(Initial.streams_after());
    assert!(Never.streams_after());
    assert!(WhenNeeded.streams_after());
    assert!(SchemaOnly.streams_after());
    assert!(
        !InitialOnly.streams_after(),
        "initial_only exits after snapshot"
    );
    assert!(
        !SchemaOnlyRecovery.streams_after(),
        "schema_only_recovery exits after recovery"
    );
}

/// Cite: debezium `IncrementalSnapshotContext::nextChunkId` — chunks
/// advance monotonically; each completion bumps `chunks_completed`
/// and updates the watermark pair.
#[test]
fn snapshot_progress_chunk_completion_advances_watermarks() {
    let mut p = SnapshotProgress::new(TENANT, "billing.public.orders", SnapshotMode::Initial);
    p.chunks_total = Some(3);
    assert_eq!(p.chunks_completed, 0);
    assert_eq!(p.percent(), Some(0.0));

    p.complete_chunk(serde_json::json!(0), serde_json::json!(1000))
        .unwrap();
    assert_eq!(p.chunks_completed, 1);
    assert!((p.percent().unwrap() - 33.333).abs() < 0.1);

    p.complete_chunk(serde_json::json!(1000), serde_json::json!(2000))
        .unwrap();
    p.complete_chunk(serde_json::json!(2000), serde_json::json!(3000))
        .unwrap();
    assert_eq!(p.chunks_completed, 3);
    assert!(p.completed, "auto-completes when chunks_completed >= total");
    assert_eq!(p.percent(), Some(100.0));

    // Once completed, further chunks are rejected.
    let err = p
        .complete_chunk(serde_json::json!(3000), serde_json::json!(4000))
        .unwrap_err();
    assert!(matches!(err, CdcError::InvalidConfig(_)));
}

/// Cite: debezium `IncrementalSnapshotContext` — when `chunks_total`
/// is unknown (e.g. live table where the planner cannot enumerate
/// chunks ahead of time), `percent()` returns None and the operator
/// must call `mark_complete()` explicitly when done.
#[test]
fn snapshot_progress_with_unknown_total_requires_explicit_complete() {
    let mut p = SnapshotProgress::new(TENANT, "shop.public.events", SnapshotMode::WhenNeeded);
    assert!(p.chunks_total.is_none());
    assert!(p.percent().is_none(), "unbounded ⇒ no percent");

    for i in 0..5u64 {
        p.complete_chunk(serde_json::json!(i * 100), serde_json::json!((i + 1) * 100))
            .unwrap();
    }
    assert_eq!(p.chunks_completed, 5);
    assert!(!p.completed, "unbounded snapshot doesn't auto-complete");

    p.mark_complete();
    assert!(p.completed);
}

/// Cite: debezium `SnapshotChangeEventSource` watermark serialisation
/// — progress structs round-trip through JSON.
#[test]
fn snapshot_progress_serde_round_trip() {
    let mut p = SnapshotProgress::new(TENANT, "billing.orders", SnapshotMode::Initial);
    p.chunks_total = Some(10);
    p.complete_chunk(serde_json::json!("01"), serde_json::json!("02"))
        .unwrap();

    let json = serde_json::to_string(&p).unwrap();
    let back: SnapshotProgress = serde_json::from_str(&json).unwrap();
    assert_eq!(back.tenant_id, TENANT);
    assert_eq!(back.table_id, "billing.orders");
    assert_eq!(back.mode, SnapshotMode::Initial);
    assert_eq!(back.chunks_completed, 1);
    assert_eq!(back.last_low_watermark, serde_json::json!("01"));
    assert_eq!(back.last_high_watermark, serde_json::json!("02"));
}

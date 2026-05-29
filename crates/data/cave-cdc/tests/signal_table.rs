// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Signal table tests — debezium signal API for incremental snapshots
//! and ad-hoc schema changes.
//!
//! Cite: debezium docs "Sending signals to a Debezium connector" +
//! `debezium-core SignalRecord` + `ExecuteSnapshot` signal.

use cave_cdc::signal::{Signal, SignalKind, SignalTable};

const TENANT: &str = "acme-test";

#[test]
fn signal_table_accepts_execute_snapshot_signal() {
    let mut tbl = SignalTable::new(TENANT, "signal.table_signals");
    let sig = Signal {
        id: "sig-001".into(),
        signal_type: SignalKind::ExecuteSnapshot,
        data: serde_json::json!({"data-collections": ["public.orders"]}),
    };
    tbl.push(sig).unwrap();
    assert_eq!(tbl.pending_count(), 1);
}

#[test]
fn signal_table_rejects_duplicate_id() {
    let mut tbl = SignalTable::new(TENANT, "signal.table_signals");
    let sig = Signal {
        id: "dup".into(),
        signal_type: SignalKind::ExecuteSnapshot,
        data: serde_json::json!({}),
    };
    tbl.push(sig.clone()).unwrap();
    let err = tbl.push(sig).unwrap_err();
    assert!(err.to_string().contains("dup"), "error should mention the duplicate id");
}

#[test]
fn signal_table_rejects_empty_id() {
    let mut tbl = SignalTable::new(TENANT, "signal.table_signals");
    let sig = Signal {
        id: "".into(),
        signal_type: SignalKind::ExecuteSnapshot,
        data: serde_json::json!({}),
    };
    let err = tbl.push(sig).unwrap_err();
    assert!(err.to_string().contains("id"), "error should mention id");
}

#[test]
fn signal_table_drain_consumes_pending_signals() {
    let mut tbl = SignalTable::new(TENANT, "signal.table_signals");
    for i in 0..3 {
        tbl.push(Signal {
            id: format!("sig-{}", i),
            signal_type: SignalKind::ExecuteSnapshot,
            data: serde_json::json!({"idx": i}),
        })
        .unwrap();
    }
    let drained = tbl.drain();
    assert_eq!(drained.len(), 3);
    assert_eq!(tbl.pending_count(), 0);
}

#[test]
fn signal_kind_log_signal_variant() {
    let mut tbl = SignalTable::new(TENANT, "signal.table_signals");
    let sig = Signal {
        id: "log-001".into(),
        signal_type: SignalKind::Log,
        data: serde_json::json!({"message": "hello"}),
    };
    tbl.push(sig).unwrap();
    assert_eq!(tbl.pending_count(), 1);
}

#[test]
fn signal_serde_round_trip() {
    let sig = Signal {
        id: "rt-001".into(),
        signal_type: SignalKind::ExecuteSnapshot,
        data: serde_json::json!({"data-collections": ["public.orders"]}),
    };
    let json = serde_json::to_string(&sig).unwrap();
    let back: Signal = serde_json::from_str(&json).unwrap();
    assert_eq!(sig, back);
}

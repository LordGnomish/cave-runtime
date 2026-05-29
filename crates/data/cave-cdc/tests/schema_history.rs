// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Schema history tests — DDL journal for connector restart recovery.
//!
//! Cite: debezium-storage `io.debezium.storage.SchemaHistory` +
//! `io.debezium.relational.history.MemorySchemaHistory` +
//! `HistoryRecord` shape.

use cave_cdc::schema_history::{HistoryRecord, HistorySource, SchemaHistory};

const TENANT: &str = "acme-hist-test";

fn make_record(idx: u32) -> HistoryRecord {
    HistoryRecord {
        tenant_id: TENANT.into(),
        source: HistorySource {
            connector: "pg-connector".into(),
            db: "billing".into(),
            schema: "public".into(),
            ts_ms: 1_700_000_000_000 + idx as i64,
        },
        ddl: format!("CREATE TABLE orders_{} (id BIGINT PRIMARY KEY)", idx),
        table_changes: vec![format!("public.orders_{}", idx)],
    }
}

#[test]
fn schema_history_append_and_count() {
    let mut hist = SchemaHistory::new(TENANT);
    hist.record(make_record(1)).unwrap();
    hist.record(make_record(2)).unwrap();
    assert_eq!(hist.len(), 2);
}

#[test]
fn schema_history_rejects_cross_tenant_record() {
    let mut hist = SchemaHistory::new(TENANT);
    let mut r = make_record(1);
    r.tenant_id = "other-tenant".into();
    let err = hist.record(r).unwrap_err();
    assert!(err.to_string().contains("cross-tenant"), "should be cross-tenant: {}", err);
}

#[test]
fn schema_history_filter_by_table() {
    let mut hist = SchemaHistory::new(TENANT);
    hist.record(make_record(1)).unwrap();
    hist.record(make_record(2)).unwrap();
    let found = hist.records_for_table("public.orders_1");
    assert_eq!(found.len(), 1);
    assert!(found[0].ddl.contains("orders_1"));
}

#[test]
fn schema_history_records_since_ts() {
    let mut hist = SchemaHistory::new(TENANT);
    hist.record(make_record(1)).unwrap(); // ts = 1_700_000_000_001
    hist.record(make_record(2)).unwrap(); // ts = 1_700_000_000_002
    hist.record(make_record(3)).unwrap(); // ts = 1_700_000_000_003
    // Fetch records after ts of record 1
    let after = hist.records_since_ts(1_700_000_000_001);
    assert_eq!(after.len(), 2, "should have records for idx 2 and 3");
}

#[test]
fn schema_history_serde_round_trip() {
    let r = make_record(42);
    let json = serde_json::to_string(&r).unwrap();
    let back: HistoryRecord = serde_json::from_str(&json).unwrap();
    assert_eq!(r, back);
}

#[test]
fn schema_history_empty_ddl_rejected() {
    let mut hist = SchemaHistory::new(TENANT);
    let mut r = make_record(1);
    r.ddl = "".into();
    let err = hist.record(r).unwrap_err();
    assert!(err.to_string().contains("ddl"), "should mention ddl: {}", err);
}

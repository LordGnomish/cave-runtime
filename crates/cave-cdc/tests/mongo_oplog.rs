//! cave-cdc — MongoDB oplog tests.
//! Pinned to debezium-connector-mongodb v3.5.0.Final.

use cave_cdc::mongo::{OplogEvent, OplogOp, ResumeToken};
use cave_cdc::{CdcError, MongoDbConnector, SourceConnector};

const TENANT: &str = "tenant-acme-prod";

/// Cite: MongoDB `oplog.rs` `op` field — `i/u/d/c/n` byte codes round
/// trip through `OplogOp::from_str` / `as_str`.
#[test]
fn oplog_op_byte_codes_round_trip() {
    for (s, op) in [
        ("i", OplogOp::Insert),  ("u", OplogOp::Update),
        ("d", OplogOp::Delete),  ("c", OplogOp::Command),
        ("n", OplogOp::Noop),
    ] {
        let parsed = OplogOp::from_str(s).unwrap();
        assert_eq!(parsed, op);
        assert_eq!(parsed.as_str(), s);
    }
    assert!(OplogOp::from_str("z").is_none());
    let _ = TENANT;
}

/// Cite: debezium-connector-mongodb namespace parsing — `<db>.<col>`
/// where the FIRST `.` is the separator. Empty halves are illegal.
#[test]
fn oplog_event_namespace_split_at_first_dot() {
    let ev = OplogEvent {
        op: OplogOp::Insert,
        namespace: "shop.orders".into(),
        document_id: serde_json::json!({ "_id": 42 }),
        before: None,
        after: Some(serde_json::json!({ "_id": 42, "tenant_id": TENANT })),
        cluster_time_secs: 1_700_000_000,
        resume_token: ResumeToken::new("ABCD", 1_700_000_000, 1),
    };
    let (db, col) = ev.db_and_collection().unwrap();
    assert_eq!(db, "shop");
    assert_eq!(col, "orders");

    // Collections may contain dots (system.indexes) — split at FIRST.
    let mut ev2 = ev.clone();
    ev2.namespace = "shop.system.indexes".into();
    let (db, col) = ev2.db_and_collection().unwrap();
    assert_eq!(db, "shop");
    assert_eq!(col, "system.indexes");

    // Empty halves rejected.
    let mut bad = ev.clone();
    bad.namespace = ".orders".into();
    assert!(bad.db_and_collection().is_err());
    bad.namespace = "shop.".into();
    assert!(bad.db_and_collection().is_err());
}

/// Cite: debezium-connector-mongodb `MongoDbStreamingChangeEventSource`
/// `include_namespaces` filter — empty list = all; non-empty = exact
/// match per `<db>.<col>`.
#[test]
fn include_namespaces_gates_emission() {
    let mut c = MongoDbConnector::new(format!("{}-mongo", TENANT), TENANT, "rs0");
    assert!(c.should_emit("shop.orders"));
    c.include_namespaces = vec!["shop.orders".into(), "shop.customers".into()];
    assert!(c.should_emit("shop.orders"));
    assert!(c.should_emit("shop.customers"));
    assert!(!c.should_emit("audit.events"));
}

/// Cite: MongoDB change-streams docs — resume tokens advance
/// monotonically. cave rejects regressions because they signal a
/// desynchronised checkpoint.
#[test]
fn resume_token_monotonic_advancement() {
    let mut c = MongoDbConnector::new(format!("{}-mongo", TENANT), TENANT, "rs0");
    c.start().unwrap();

    let t1 = ResumeToken::new("AAA", 1_700_000_000, 1);
    let t2 = ResumeToken::new("AAA", 1_700_000_000, 2);
    let t3 = ResumeToken::new("AAA", 1_700_000_001, 1);
    c.record_resume_token(t1.clone()).unwrap();
    c.record_resume_token(t2.clone()).unwrap();
    c.record_resume_token(t3.clone()).unwrap();

    assert_eq!(c.last_resume_token(), Some(&t3));

    let regress = c.record_resume_token(t1).unwrap_err();
    assert!(matches!(regress, CdcError::InvalidConfig(_)));
}

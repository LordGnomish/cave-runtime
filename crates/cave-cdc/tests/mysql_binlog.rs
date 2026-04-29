//! cave-cdc — MySQL binlog tests.
//! Pinned to debezium-connector-mysql v3.5.0.Final.

use cave_cdc::mysql::{BinlogPosition, BinlogEventType};
use cave_cdc::{CdcError, MySqlConnector, SourceConnector};

const TENANT: &str = "tenant-acme-prod";

/// Cite: MySQL `log_event.h::Log_event_type` numeric codes — the
/// canonical mapping must round-trip through `type_code` /
/// `from_code`.
#[test]
fn binlog_event_type_codes_match_mysql_log_event_h() {
    use BinlogEventType::*;
    let table = [
        (Query, 2), (Rotate, 4), (FormatDescription, 15),
        (Xid, 16), (TableMap, 19), (WriteRows, 30),
        (UpdateRows, 31), (DeleteRows, 32), (Gtid, 33),
    ];
    for (kind, code) in table {
        assert_eq!(kind.type_code(), code);
        assert_eq!(BinlogEventType::from_code(code), Some(kind));
    }
    assert!(BinlogEventType::from_code(99).is_none());
    let _ = TENANT;
}

/// Cite: debezium-connector-mysql `MySqlStreamingChangeEventSource` —
/// only WRITE/UPDATE/DELETE row events generate downstream
/// ChangeEvents. Bookkeeping events (Rotate, Xid, TableMap, …) are
/// silently consumed.
#[test]
fn only_row_events_emit_downstream_change_events() {
    use BinlogEventType::*;
    let row_events = [WriteRows, UpdateRows, DeleteRows];
    let bookkeeping = [Query, Rotate, FormatDescription, Xid, TableMap, Gtid];
    for k in row_events {
        assert!(k.is_row_event(), "{:?} is a row event", k);
    }
    for k in bookkeeping {
        assert!(!k.is_row_event(), "{:?} is bookkeeping", k);
    }
}

/// Cite: MySQL binlog file naming `<basename>.<6-digit-seq>` — the
/// suffix must be purely digits; non-digit suffixes are rejected.
#[test]
fn binlog_position_validates_canonical_filename() {
    assert!(BinlogPosition { file: "mysql-bin.000123".into(), pos: 4 }.validate().is_ok());
    assert!(BinlogPosition { file: "binlog.000001".into(), pos: 0 }.validate().is_ok());

    // Missing dot
    assert!(BinlogPosition { file: "no-dot".into(), pos: 0 }.validate().is_err());
    // Non-digit suffix
    assert!(BinlogPosition { file: "binlog.abcdef".into(), pos: 0 }.validate().is_err());
    // Empty file
    assert!(BinlogPosition { file: "".into(), pos: 0 }.validate().is_err());
}

/// Cite: debezium-connector-mysql `MySqlStreamingChangeEventSource`
/// — `include_schemas` filter (when non-empty) gates which schemas
/// emit downstream events.
#[test]
fn include_schemas_filter_gates_downstream_emission() {
    let mut c = MySqlConnector::new(
        format!("{}-mysql", TENANT), TENANT, "shop", 1042);
    // Empty allow list ⇒ everything passes.
    assert!(c.should_emit("billing"));
    assert!(c.should_emit("audit"));

    c.include_schemas = vec!["billing".into(), "orders".into()];
    assert!(c.should_emit("billing"));
    assert!(c.should_emit("orders"));
    assert!(!c.should_emit("audit"), "audit not in allow-list ⇒ filtered out");
}

/// Cite: debezium-connector-mysql `GtidSet` + `MySqlOffsetContext` —
/// the connector remembers the last committed binlog position and the
/// last GTID so restarts resume from the right point.
#[test]
fn record_position_persists_checkpoint_and_gtid() {
    let mut c = MySqlConnector::new(
        format!("{}-mysql", TENANT), TENANT, "shop", 1042);
    c.start().unwrap();

    let pos = BinlogPosition { file: "mysql-bin.000007".into(), pos: 4096 };
    c.record_position(pos.clone(), Some("3E11FA47-71CA-11E1-9E33-C80AA9429562:23".into())).unwrap();
    assert_eq!(c.last_committed_position(), &pos);
    assert_eq!(c.last_gtid(), Some("3E11FA47-71CA-11E1-9E33-C80AA9429562:23"));

    // Bad position is rejected — checkpoint not advanced.
    let bad = BinlogPosition { file: "no-dot".into(), pos: 0 };
    let err = c.record_position(bad, None).unwrap_err();
    assert!(matches!(err, CdcError::InvalidBinlogPosition { .. }));
    assert_eq!(c.last_committed_position(), &pos, "checkpoint unchanged on rejection");
}

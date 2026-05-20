// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! cave-cdc — Postgres logical replication tests.
//! Pinned to debezium-connector-postgres v3.5.0.Final.

use cave_cdc::postgres::{DecodingPlugin, Lsn, ReplicationSlotConfig, WalEventKind};
use cave_cdc::{CdcError, PostgresConnector, SourceConnector};

const TENANT: &str = "tenant-acme-prod";

fn slot() -> ReplicationSlotConfig {
    ReplicationSlotConfig {
        slot_name: format!("{}_billing", TENANT.replace('-', "_")),
        publication_name: format!("{}_billing_pub", TENANT.replace('-', "_")),
        plugin: DecodingPlugin::Pgoutput,
        drop_slot_on_stop: false,
    }
}

/// Cite: Postgres `pg_lsn` text format — uppercase hex pair joined
/// by `/`. cave parses both `XXX/XXX` and bare hex.
#[test]
fn lsn_parse_canonical_and_bare_hex_round_trip() {
    let lsn = Lsn::parse("16/B374D848").unwrap();
    assert_eq!(lsn.0, 0x16_B374D848);
    assert_eq!(lsn.as_text(), "16/B374D848");

    // Bare hex (Postgres also accepts this for tooling)
    let lsn = Lsn::parse("ABCDEF").unwrap();
    assert_eq!(lsn.0, 0xABCDEF);

    let err = Lsn::parse("not-an-lsn").unwrap_err();
    assert!(matches!(err, CdcError::InvalidLsn(_, _)));
}

/// Cite: debezium-connector-postgres `PostgresConnector::validate` —
/// slot + publication names must be lowercase identifiers (Postgres
/// folds unquoted names) ≤ 63 bytes with `[a-z0-9_]` only.
#[test]
fn replication_slot_validates_postgres_identifier_rules() {
    let mut s = slot();
    assert!(s.validate().is_ok());

    s.slot_name = "MixedCase".into();
    assert!(s.validate().is_err(), "uppercase rejected");

    s.slot_name = "x".repeat(64);
    assert!(s.validate().is_err(), "exceeds NAMEDATALEN-1");

    s.slot_name = "has space".into();
    assert!(s.validate().is_err(), "whitespace rejected");

    s.slot_name = "valid_slot_name".into();
    s.publication_name = "".into();
    assert!(s.validate().is_err(), "empty publication rejected");
}

/// Cite: debezium-connector-postgres
/// `PostgresReplicationConnection::sendStandbyStatusUpdate` — the
/// flushed LSN must advance monotonically; a regression is a bug
/// upstream and cave rejects it explicitly.
#[test]
fn flush_lsn_must_advance_monotonically() {
    let mut c = PostgresConnector::new(format!("{}-pg", TENANT), TENANT, "billing", slot());
    c.start().unwrap();
    assert_eq!(c.last_flushed_lsn(), Lsn(0));

    c.flush_lsn(Lsn::parse("0/100").unwrap()).unwrap();
    c.flush_lsn(Lsn::parse("0/200").unwrap()).unwrap();
    assert_eq!(c.last_flushed_lsn(), Lsn(0x200));

    let err = c.flush_lsn(Lsn::parse("0/100").unwrap()).unwrap_err();
    assert!(matches!(err, CdcError::InvalidLsn(_, _)));
}

/// Cite: debezium-connector-postgres `MessageType` — ten pgoutput
/// message kinds; the streaming source dispatches on this enum.
#[test]
fn wal_event_kinds_serde_round_trip() {
    use WalEventKind::*;
    for k in [
        Begin, Commit, Insert, Update, Delete, Truncate, Relation, Type, Origin, Message,
    ] {
        let json = serde_json::to_string(&k).unwrap();
        let back: WalEventKind = serde_json::from_str(&json).unwrap();
        assert_eq!(k, back);
    }
}

/// Cite: debezium-connector-postgres `BaseSourceTask::doStart` —
/// `flush_lsn` MUST NOT be callable before `start()`.
#[test]
fn flush_before_start_returns_not_connected() {
    let mut c = PostgresConnector::new(format!("{}-pg", TENANT), TENANT, "billing", slot());
    let err = c.flush_lsn(Lsn(0x100)).unwrap_err();
    assert!(matches!(err, CdcError::NotConnected(_)));
}

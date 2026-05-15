// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! cave-cdc — SourceConnector trait + ChangeEvent shape tests.
//! Pinned to debezium v3.5.0.Final.

use cave_cdc::{
    ChangeOperation, ConnectorState, MySqlConnector, PostgresConnector, SourceConnector,
    ReplicationSlotConfig,
};
use cave_cdc::postgres::DecodingPlugin;
use cave_cdc::CdcError;

const TENANT: &str = "tenant-acme-prod";

fn pg() -> PostgresConnector {
    PostgresConnector::new(
        format!("{}-pg", TENANT), TENANT, "billing",
        ReplicationSlotConfig {
            slot_name: format!("{}_billing", TENANT.replace('-', "_")),
            publication_name: format!("{}_billing_pub", TENANT.replace('-', "_")),
            plugin: DecodingPlugin::Pgoutput,
            drop_slot_on_stop: false,
        },
    )
}

/// Cite: debezium `Envelope.Operation` — wire codes c/u/d/r/t/m must
/// round-trip through `ChangeOperation::from_str` / `as_str`.
#[test]
fn change_operation_wire_codes_round_trip() {
    for (s, op) in [
        ("c", ChangeOperation::Create), ("u", ChangeOperation::Update),
        ("d", ChangeOperation::Delete), ("r", ChangeOperation::Read),
        ("t", ChangeOperation::Truncate), ("m", ChangeOperation::Message),
    ] {
        let parsed = ChangeOperation::from_str(s).unwrap();
        assert_eq!(parsed, op);
        assert_eq!(parsed.as_str(), s);
    }
    assert!(ChangeOperation::from_str("x").is_none());
    let _ = TENANT;
}

/// Cite: debezium `BaseSourceTask::State` — Initial → Snapshotting →
/// Streaming → Stopped is the canonical lifecycle. Self-loops are
/// idempotent; backwards transitions are forbidden.
#[test]
fn connector_state_machine_legal_transitions() {
    use ConnectorState::*;
    assert!(Initial.can_transition_to(Snapshotting));
    assert!(Initial.can_transition_to(Streaming));   // skip-snapshot mode
    assert!(Snapshotting.can_transition_to(Streaming));
    assert!(Streaming.can_transition_to(Stopped));
    assert!(Streaming.can_transition_to(Failed));
    assert!(Streaming.can_transition_to(Streaming), "self-loop is idempotent");

    // Forbidden: backward edges + reviving terminals.
    assert!(!Streaming.can_transition_to(Initial));
    assert!(!Stopped.can_transition_to(Streaming));
    assert!(!Stopped.can_transition_to(Initial));
}

/// Cite: debezium `BaseSourceTask::doStart`/`doStop` lifecycle — start
/// promotes Initial → Streaming (via Snapshotting); start on a
/// running connector is rejected; stop is idempotent.
#[test]
fn postgres_connector_lifecycle_start_then_stop() {
    let mut c = pg();
    assert_eq!(c.state(), ConnectorState::Initial);
    assert_eq!(c.tenant_id(), TENANT);
    assert!(c.validate().is_ok());

    c.start().unwrap();
    assert_eq!(c.state(), ConnectorState::Streaming);

    let err = c.start().unwrap_err();
    assert_eq!(err, CdcError::AlreadyRunning);

    c.stop().unwrap();
    assert_eq!(c.state(), ConnectorState::Stopped);

    let mut idle = pg();
    assert!(idle.stop().is_ok(), "stop from Initial is a no-op");
}

/// Cite: debezium `MySqlConnector::validate` — `server_id` MUST be
/// non-zero (MySQL replication identity).
#[test]
fn mysql_connector_validates_server_id_and_tenant() {
    let mut bad = MySqlConnector::new(
        format!("{}-mysql", TENANT), TENANT, "shop", 0);
    assert!(bad.validate().is_err());

    bad.server_id = 1042;
    assert!(bad.validate().is_ok());

    let mut empty_tenant = MySqlConnector::new(
        "no-tenant", "", "shop", 1042);
    assert!(empty_tenant.validate().is_err());

    let mut good = MySqlConnector::new(
        format!("{}-mysql", TENANT), TENANT, "shop", 1042);
    assert!(good.start().is_ok());
    assert_eq!(good.state(), ConnectorState::Streaming);
}

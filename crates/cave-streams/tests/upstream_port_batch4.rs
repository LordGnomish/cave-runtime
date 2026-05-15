//! Batch 4 (2026-05-14) — Kafka Connect REST admin API port.
//!
//! Batch3 covered the Connect runtime state machine (connector/task
//! lifecycle, distributed herder semantics). Batch4 extends that surface
//! to the public REST admin contract: list/create/restart/pause/resume/
//! delete/validate/status endpoints exposed by Kafka's
//! `ConnectorsResource`. The tests assert dispatch-layer responses
//! (status codes + typed bodies + Location header) rather than wiring
//! axum HTTP — matching how cave-streams structures its other
//! batched ports.
//!
//! Upstream: apache/kafka @ 4.2.0
//!   * connect/runtime/src/test/java/org/apache/kafka/connect/runtime/rest/resources/ConnectorsResourceTest.java
//!   * connect/runtime/src/main/java/org/apache/kafka/connect/runtime/rest/resources/ConnectorsResource.java
//!   * connect/runtime/src/main/java/org/apache/kafka/connect/runtime/rest/resources/RootResource.java
//!
//! Tag `4.2.0` selected to align with `parity.manifest.toml::upstream.version`
//! (verified present in https://api.github.com/repos/apache/kafka/tags
//! — `4.2.1-rc5`, `4.2.0` and `4.3.0-rc2` all current).
//!
//! Honest deferral: `RestServerTest.java` was reorganised between 4.1
//! and 4.2 and is no longer at the path documented in the batch
//! preamble (the 4.2.0 raw fetch returns 404). The relevant assertions
//! (advertised URL formation + error envelope) live in newer
//! `ConnectRestServerTest` + `RestExceptionMapper` files; porting the
//! HTTP-server framing layer (jersey/jetty) is out of scope for the
//! state-machine port.  ServerInfo (RootResource) is still covered here
//! because the body is a plain DTO.

use cave_streams::connect_rest::{
    AdminError, ConfigInfo, ConnectRestAdmin, ConnectorStatus, CreateError, CreatedConnector,
    HttpStatus, RestartError, RestartReport, ServerInfo, ValidationResult,
};
use std::collections::HashMap;

fn jdbc_source(name: &str) -> HashMap<String, String> {
    let mut c = HashMap::new();
    c.insert(
        "connector.class".into(),
        "cave.connect.JdbcSourceConnector".into(),
    );
    c.insert("tasks.max".into(), "2".into());
    c.insert("connection.url".into(), "jdbc:postgresql://h/db".into());
    c.insert("topic.prefix".into(), "db-".into());
    c.insert("name".into(), name.into());
    c
}

fn config_without_name() -> HashMap<String, String> {
    let mut c = HashMap::new();
    c.insert(
        "connector.class".into(),
        "cave.connect.JdbcSourceConnector".into(),
    );
    c.insert("tasks.max".into(), "1".into());
    c
}

// ────────────────────────────────────────────────────────────────────────────
// ConnectorsResourceTest — list / expand
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: ConnectorsResourceTest / `testListConnectors`
/// (lines 212-220). Empty admin returns empty list; populated admin
/// returns connector names. The dispatch layer answers with 200 OK
/// and a JSON list (`herder.connectors()`).
#[test]
fn upstream_rest_list_connectors_returns_alphabetical_names() {
    let mut admin = ConnectRestAdmin::new();
    assert!(admin.list_connectors().is_empty());

    admin
        .create_connector(Some("zeta".into()), jdbc_source("zeta"))
        .unwrap();
    admin
        .create_connector(Some("alpha".into()), jdbc_source("alpha"))
        .unwrap();
    admin
        .create_connector(Some("mike".into()), jdbc_source("mike"))
        .unwrap();

    let names = admin.list_connectors();
    assert_eq!(names, vec!["alpha", "mike", "zeta"]);
}

// ────────────────────────────────────────────────────────────────────────────
// ConnectorsResourceTest — create
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: ConnectorsResourceTest / `testCreateConnector`
/// (lines 298-308). Successful create returns 201 Created with a
/// Location header pointing at `/connectors/{name}` and a body
/// containing the resulting `ConnectorInfo`.
#[test]
fn upstream_rest_create_connector_returns_201_with_location() {
    let mut admin = ConnectRestAdmin::new();
    let created: CreatedConnector = admin
        .create_connector(Some("test".into()), jdbc_source("test"))
        .unwrap();
    assert_eq!(created.status, HttpStatus::Created);
    assert_eq!(created.location, "/connectors/test");
    assert_eq!(created.info.name, "test");
    assert_eq!(created.info.tasks.len(), 2);
}

/// Upstream: ConnectorsResourceTest / `testCreateConnectorExists`
/// (lines 372-381). Duplicate name maps to `AlreadyExistsException`
/// → REST envelope 409 Conflict.
#[test]
fn upstream_rest_create_connector_duplicate_returns_409_conflict() {
    let mut admin = ConnectRestAdmin::new();
    admin
        .create_connector(Some("dup".into()), jdbc_source("dup"))
        .unwrap();
    let err = admin
        .create_connector(Some("dup".into()), jdbc_source("dup"))
        .unwrap_err();
    assert!(matches!(err, CreateError::Conflict(_)));
    assert_eq!(err.status(), HttpStatus::Conflict);
}

/// Upstream: ConnectorsResourceTest / `testCreateConnectorNameTrimWhitespaces`
/// (lines 383-398). Padding whitespace in the name field is trimmed
/// before reaching the herder; "   test  \n  " → "test".
#[test]
fn upstream_rest_create_connector_trims_padding_whitespace_from_name() {
    let mut admin = ConnectRestAdmin::new();
    let created = admin
        .create_connector(Some("   test  \n  ".into()), config_without_name())
        .unwrap();
    assert_eq!(created.info.name, "test");
    // Name is materialised into the config map under the upstream
    // `ConnectorConfig.NAME_CONFIG` key.
    assert_eq!(
        created.info.config.get("name").map(String::as_str),
        Some("test")
    );
    assert_eq!(created.location, "/connectors/test");
}

/// Upstream: ConnectorsResourceTest / `testCreateConnectorNameAllWhitespaces`
/// (lines 400-415). Whitespace-only name normalises to "".  Together
/// with `testCreateConnectorNoName`, this is upstream's "null name →
/// empty string" contract.
#[test]
fn upstream_rest_create_connector_all_whitespace_name_normalises_to_empty() {
    let mut admin = ConnectRestAdmin::new();
    let created = admin
        .create_connector(Some("    \n\t  ".into()), config_without_name())
        .unwrap();
    assert_eq!(created.info.name, "");
    assert_eq!(
        created.info.config.get("name").map(String::as_str),
        Some("")
    );
}

/// Upstream: ConnectorsResourceTest / `testCreateConnectorNoName`
/// (lines 417-432). `null` name in the request body normalises to "".
#[test]
fn upstream_rest_create_connector_null_name_normalises_to_empty() {
    let mut admin = ConnectRestAdmin::new();
    let created = admin
        .create_connector(None, config_without_name())
        .unwrap();
    assert_eq!(created.info.name, "");
    assert_eq!(
        created.info.config.get("name").map(String::as_str),
        Some("")
    );
}

/// Upstream: ConnectorsResourceTest / `testCreateConnectorConfigNameMismatch`
/// (lines 533-540). The request body's name field must match the
/// `name` key inside the supplied config — otherwise 400 Bad Request
/// (`BadRequestException`).
#[test]
fn upstream_rest_create_connector_config_name_mismatch_returns_400() {
    let mut admin = ConnectRestAdmin::new();
    let mut config = config_without_name();
    config.insert("name".into(), "mismatched-name".into());
    let err = admin
        .create_connector(Some("real-name".into()), config)
        .unwrap_err();
    assert!(matches!(err, CreateError::InvalidName(_)));
    assert_eq!(err.status(), HttpStatus::BadRequest);
}

// ────────────────────────────────────────────────────────────────────────────
// ConnectorsResourceTest — restart
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: ConnectorsResourceTest / `testRestartConnectorAndTasksRequestAccepted`
/// (lines 619-633). Successful restart of connector+tasks returns
/// 202 Accepted; entity carries `ConnectorStateInfo` with state
/// `RESTARTING`.
#[test]
fn upstream_rest_restart_connector_and_tasks_returns_202_accepted_with_state_info() {
    let mut admin = ConnectRestAdmin::new();
    admin
        .create_connector(Some("foo".into()), jdbc_source("foo"))
        .unwrap();
    let report: RestartReport = admin
        .restart_connector("foo", /* include_tasks */ true, /* only_failed */ false)
        .unwrap();
    assert_eq!(report.status, HttpStatus::Accepted);
    assert_eq!(report.state_info.name, "foo");
    assert_eq!(report.state_info.connector_state, "RESTARTING");
}

/// Upstream: ConnectorsResourceTest / `testRestartConnectorAndTasksConnectorNotFound`
/// (lines 584-593). Restart of an unknown connector throws
/// `NotFoundException` → 404 Not Found.
#[test]
fn upstream_rest_restart_connector_not_found_returns_404() {
    let mut admin = ConnectRestAdmin::new();
    let err = admin.restart_connector("ghost", true, false).unwrap_err();
    assert!(matches!(err, RestartError::NotFound(_)));
    assert_eq!(err.status(), HttpStatus::NotFound);
}

// ────────────────────────────────────────────────────────────────────────────
// ConnectorsResourceTest — pause / resume / delete / status
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: ConnectorsResource / `pauseConnector` (PUT) — idempotent,
/// returns 202 Accepted.  Unknown connector → 404.  The herder
/// translates these into pauseConnector calls, see
/// ConnectorsResourceTest stubbing of the same method.
#[test]
fn upstream_rest_pause_connector_idempotent_returns_unit_or_404() {
    let mut admin = ConnectRestAdmin::new();
    admin
        .create_connector(Some("p".into()), jdbc_source("p"))
        .unwrap();
    assert!(admin.pause_connector("p").is_ok());
    // Idempotent — second pause is still ok.
    assert!(admin.pause_connector("p").is_ok());

    let err = admin.pause_connector("missing").unwrap_err();
    assert!(matches!(err, AdminError::NotFound(_)));
    assert_eq!(err.status(), HttpStatus::NotFound);
}

/// Upstream: ConnectorsResource / `resumeConnector` (PUT) — idempotent
/// 202 Accepted; symmetric NotFound behaviour with pause.
#[test]
fn upstream_rest_resume_connector_after_pause_resumes_running() {
    let mut admin = ConnectRestAdmin::new();
    admin
        .create_connector(Some("r".into()), jdbc_source("r"))
        .unwrap();
    admin.pause_connector("r").unwrap();
    assert!(admin.resume_connector("r").is_ok());
    let status = admin.connector_status("r").unwrap();
    assert_eq!(status.connector_state, "RUNNING");
}

/// Upstream: ConnectorsResourceTest / `testDeleteConnector` (434-439) +
/// `testDeleteConnectorNotFound` (452-460). Delete returns 204 No
/// Content; unknown → 404 NotFoundException.
#[test]
fn upstream_rest_delete_connector_returns_204_or_404() {
    let mut admin = ConnectRestAdmin::new();
    admin
        .create_connector(Some("d".into()), jdbc_source("d"))
        .unwrap();
    assert!(admin.delete_connector("d").is_ok());
    assert!(admin.list_connectors().is_empty());

    let err = admin.delete_connector("ghost").unwrap_err();
    assert!(matches!(err, AdminError::NotFound(_)));
    assert_eq!(err.status(), HttpStatus::NotFound);
}

/// Upstream: ConnectorsResourceTest / `testGetConnectorConfigConnectorNotFound`
/// (482-490) + `getConnectorStatus` 200 OK path.  Missing connector
/// status → `None` (which the REST layer turns into 404).
#[test]
fn upstream_rest_connector_status_present_or_none() {
    let mut admin = ConnectRestAdmin::new();
    admin
        .create_connector(Some("s".into()), jdbc_source("s"))
        .unwrap();
    let status: ConnectorStatus = admin.connector_status("s").unwrap();
    assert_eq!(status.name, "s");
    assert_eq!(status.connector_state, "RUNNING");
    assert_eq!(status.tasks.len(), 2);
    assert!(status.tasks.iter().all(|t| t.state == "RUNNING"));

    assert!(admin.connector_status("ghost").is_none());
}

// ────────────────────────────────────────────────────────────────────────────
// ConnectorsResourceTest — validate config + Root server info
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: ConnectorPluginsResource / `validateConfigs` (200 OK with
/// per-key `ConfigInfo`). Missing `connector.class` yields a
/// validation error on that key with non-empty `errors`.
#[test]
fn upstream_rest_validate_config_reports_missing_required_keys() {
    let admin = ConnectRestAdmin::new();
    let result: ValidationResult = admin.validate_config(
        "cave.connect.JdbcSourceConnector",
        &HashMap::new(),
    );
    assert_eq!(result.status, HttpStatus::Ok);
    assert!(result.error_count >= 1);
    let class_info: &ConfigInfo = result
        .configs
        .iter()
        .find(|i| i.name == "connector.class")
        .expect("connector.class entry");
    assert!(
        !class_info.validation_errors.is_empty(),
        "missing required key surfaces validation_errors"
    );
}

/// Upstream: ConnectorPluginsResource / `validateConfigs` happy-path —
/// when all required keys are present the response is 200 OK with
/// `error_count == 0`.
#[test]
fn upstream_rest_validate_config_happy_path_reports_zero_errors() {
    let admin = ConnectRestAdmin::new();
    let result = admin
        .validate_config("cave.connect.JdbcSourceConnector", &jdbc_source("ok"));
    assert_eq!(result.error_count, 0);
    assert_eq!(result.status, HttpStatus::Ok);
    assert_eq!(result.name, "cave.connect.JdbcSourceConnector");
}

/// Upstream: RootResource / GET / (the 4.2.0 `ServerInfo` DTO returned
/// at the cluster root). Returns 200 with `version` + `commit` +
/// `kafka_cluster_id`.
#[test]
fn upstream_rest_root_server_info_exposes_kafka_cluster_id() {
    let admin = ConnectRestAdmin::with_cluster_id("test-cluster-id");
    let info: ServerInfo = admin.server_info();
    assert_eq!(info.status, HttpStatus::Ok);
    assert_eq!(info.kafka_cluster_id, "test-cluster-id");
    assert!(!info.version.is_empty());
    assert!(!info.commit.is_empty());
}

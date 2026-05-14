// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Kafka Connect REST admin state machine — pure-function dispatch
//! that mirrors the response semantics of
//! `connect/runtime/.../rest/resources/ConnectorsResource.java`.
//!
//! Each public method returns a typed `(status, body)` pair so the
//! HTTP layer (axum) can lift it directly into a `Response` without
//! re-deriving status codes. Upstream contract sources:
//!
//! - apache/kafka @ 4.2.0
//!   * `connect/runtime/.../rest/resources/ConnectorsResource.java`
//!   * `connect/runtime/.../rest/resources/RootResource.java`
//!   * `connect/runtime/.../rest/resources/ConnectorPluginsResource.java`
//!
//! Behavioural rules transcribed:
//!
//! - POST /connectors trims leading/trailing whitespace from the
//!   request `name`, replaces `null` with `""`, then mirrors the name
//!   into the config map under the `name` key. If the body already
//!   has a `name` key whose value disagrees with the request name a
//!   `BadRequestException` is raised (→ 400).
//! - POST /connectors/{c}/restart returns 202 Accepted with a
//!   `ConnectorStateInfo` whose connector state is `RESTARTING`; 404
//!   when the connector is unknown.
//! - GET /connectors lists connector names in alphabetical order.
//! - DELETE /connectors/{c} returns 204 No Content (or 404).
//! - PUT pause/resume return 202 Accepted (or 404).
//! - GET /connectors/{c}/status returns `ConnectorStateInfo` with
//!   per-task state strings.
//! - POST /connector-plugins/{class}/config/validate returns 200 OK
//!   with `ConfigInfos { name, error_count, configs }`.
//! - GET / (root) returns `ServerInfo { version, commit,
//!   kafka_cluster_id }`.

use crate::connect::ConnectCluster;
use std::collections::HashMap;

// ── HTTP-status surface ──────────────────────────────────────────────────────

/// Subset of HTTP status codes used by the Connect REST admin
/// envelope. We hold the numeric value rather than depending on
/// `http::StatusCode` to keep the state machine free of HTTP-layer
/// crates — the eventual axum mount can `From` into the real type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpStatus {
    /// 200 OK
    Ok,
    /// 201 Created (POST /connectors)
    Created,
    /// 202 Accepted (restart / pause / resume)
    Accepted,
    /// 204 No Content (DELETE)
    NoContent,
    /// 400 Bad Request (config name mismatch, validation envelopes)
    BadRequest,
    /// 404 Not Found (unknown connector)
    NotFound,
    /// 409 Conflict (duplicate create, rebalance-needed restart)
    Conflict,
}

impl HttpStatus {
    pub fn code(self) -> u16 {
        match self {
            Self::Ok => 200,
            Self::Created => 201,
            Self::Accepted => 202,
            Self::NoContent => 204,
            Self::BadRequest => 400,
            Self::NotFound => 404,
            Self::Conflict => 409,
        }
    }
}

// ── DTOs returned to the REST layer ──────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectorInfo {
    pub name: String,
    pub config: HashMap<String, String>,
    pub tasks: Vec<TaskId>,
    pub connector_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskId {
    pub connector: String,
    pub task: usize,
}

#[derive(Debug, Clone)]
pub struct CreatedConnector {
    pub status: HttpStatus,
    /// Value of the `Location:` header issued by upstream
    /// `UriBuilder.fromUri("/connectors").path(name).build()`.
    pub location: String,
    pub info: ConnectorInfo,
}

#[derive(Debug, Clone)]
pub struct RestartReport {
    pub status: HttpStatus,
    pub state_info: ConnectorStateInfo,
}

#[derive(Debug, Clone)]
pub struct ConnectorStatus {
    pub name: String,
    /// Upstream `ConnectorStateInfo.connector.state` — one of
    /// `RUNNING|PAUSED|FAILED|UNASSIGNED|RESTARTING|DESTROYED`.
    pub connector_state: String,
    pub tasks: Vec<TaskStatus>,
    pub connector_type: String,
}

#[derive(Debug, Clone)]
pub struct TaskStatus {
    pub id: usize,
    pub state: String,
    pub worker_id: String,
    pub trace: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ConnectorStateInfo {
    pub name: String,
    pub connector_state: String,
    pub tasks: Vec<TaskStatus>,
    pub connector_type: String,
}

#[derive(Debug, Clone)]
pub struct ConfigInfo {
    pub name: String,
    pub value: Option<String>,
    pub recommended_values: Vec<String>,
    pub validation_errors: Vec<String>,
    pub visible: bool,
}

#[derive(Debug, Clone)]
pub struct ValidationResult {
    pub status: HttpStatus,
    pub name: String,
    pub error_count: i32,
    pub configs: Vec<ConfigInfo>,
}

#[derive(Debug, Clone)]
pub struct ServerInfo {
    pub status: HttpStatus,
    pub version: String,
    pub commit: String,
    pub kafka_cluster_id: String,
}

// ── Typed error envelopes ────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CreateError {
    /// Duplicate connector name → 409 (upstream `AlreadyExistsException`).
    #[error("connector already exists: {0}")]
    Conflict(String),
    /// Config-name mismatch or other invalid-name shape → 400.
    #[error("bad request: {0}")]
    InvalidName(String),
}

impl CreateError {
    pub fn status(&self) -> HttpStatus {
        match self {
            Self::Conflict(_) => HttpStatus::Conflict,
            Self::InvalidName(_) => HttpStatus::BadRequest,
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RestartError {
    /// Unknown connector → 404 NotFoundException.
    #[error("connector not found: {0}")]
    NotFound(String),
    /// Herder reports a rebalance is pending → 409 (upstream
    /// `RebalanceNeededException`).
    #[error("rebalance needed: {0}")]
    RebalanceNeeded(String),
}

impl RestartError {
    pub fn status(&self) -> HttpStatus {
        match self {
            Self::NotFound(_) => HttpStatus::NotFound,
            Self::RebalanceNeeded(_) => HttpStatus::Conflict,
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AdminError {
    /// 404 — pause/resume/delete of an unknown connector.
    #[error("connector not found: {0}")]
    NotFound(String),
}

impl AdminError {
    pub fn status(&self) -> HttpStatus {
        match self {
            Self::NotFound(_) => HttpStatus::NotFound,
        }
    }
}

// ── ConnectRestAdmin — the state machine ─────────────────────────────────────

/// REST-facing admin gate over a `ConnectCluster`. Owns the dispatch
/// rules (status codes, location headers, name normalisation) so that
/// the axum handler is a thin envelope.
pub struct ConnectRestAdmin {
    cluster: ConnectCluster,
    /// Returned in the `ServerInfo` body. Upstream sources this from
    /// the controller's `kafkaClusterId()`.
    kafka_cluster_id: String,
    /// Version string baked into `ServerInfo.version`.
    version: String,
    /// Build commit baked into `ServerInfo.commit`.
    commit: String,
}

impl Default for ConnectRestAdmin {
    fn default() -> Self {
        Self::new()
    }
}

impl ConnectRestAdmin {
    pub fn new() -> Self {
        Self {
            cluster: ConnectCluster::new(),
            kafka_cluster_id: "cave-streams-cluster".into(),
            // Pinned to upstream parity audit version; `commit` is a
            // synthetic short-sha matching the cave-streams build.
            version: "4.2.0".into(),
            commit: "cave-streams".into(),
        }
    }

    pub fn with_cluster_id(id: impl Into<String>) -> Self {
        Self {
            kafka_cluster_id: id.into(),
            ..Self::new()
        }
    }

    // ── list ────────────────────────────────────────────────────────────────

    /// GET /connectors → 200 OK with a list of connector names in
    /// alphabetical order. Upstream `Herder.connectors()` is backed
    /// by a `TreeSet`-equivalent in distributed mode, so the order is
    /// part of the contract.
    pub fn list_connectors(&self) -> Vec<String> {
        let mut names = self.cluster.list_connectors();
        names.sort();
        names
    }

    // ── create ──────────────────────────────────────────────────────────────

    /// POST /connectors. Matches upstream `createConnector`:
    /// `name == null` → `""`, otherwise `name.trim()`; then mirror
    /// the normalised name into the config map under the `name` key.
    /// If the body already carries `name` and it disagrees with the
    /// normalised name, return `BadRequestException` → 400.
    /// Duplicate name → 409 Conflict.
    pub fn create_connector(
        &mut self,
        name: Option<String>,
        mut config: HashMap<String, String>,
    ) -> Result<CreatedConnector, CreateError> {
        let normalised = name.unwrap_or_default().trim().to_string();

        // BadRequestException parity: config name mismatch
        // (`checkAndPutConnectorConfigName`).
        if let Some(in_config) = config.get("name") {
            if in_config != &normalised {
                return Err(CreateError::InvalidName(format!(
                    "Connector name configuration ({}) doesn't match connector name in URI ({})",
                    in_config, normalised
                )));
            }
        }
        config.insert("name".into(), normalised.clone());

        let connector = self
            .cluster
            .create_connector(normalised.clone(), config.clone())
            .map_err(|e| {
                CreateError::Conflict(format!(
                    "connector {} already exists ({})",
                    normalised, e
                ))
            })?;

        let info = ConnectorInfo {
            name: connector.name.clone(),
            config: connector.config.clone(),
            tasks: connector
                .tasks
                .iter()
                .map(|t| TaskId {
                    connector: t.id.connector.clone(),
                    task: t.id.task,
                })
                .collect(),
            connector_type: format!("{:?}", connector.connector_type).to_uppercase(),
        };

        // Upstream URI builder: percent-encoding is the HTTP layer's
        // responsibility, but a plain path-join matches the literal
        // assertions in ConnectorsResourceTest (which compare strings
        // before encoding).
        let location = format!("/connectors/{}", normalised);

        Ok(CreatedConnector {
            status: HttpStatus::Created,
            location,
            info,
        })
    }

    // ── restart ─────────────────────────────────────────────────────────────

    /// POST /connectors/{c}/restart — returns 202 Accepted with a
    /// `ConnectorStateInfo` whose state is `RESTARTING`. Includes
    /// task entries when `include_tasks` is true; filters to failed
    /// tasks when `only_failed` is true.
    pub fn restart_connector(
        &mut self,
        name: &str,
        include_tasks: bool,
        only_failed: bool,
    ) -> Result<RestartReport, RestartError> {
        self.cluster
            .restart_connector(name)
            .map_err(|_| RestartError::NotFound(name.into()))?;

        let connector = self
            .cluster
            .get_connector(name)
            .map_err(|_| RestartError::NotFound(name.into()))?;

        let task_iter = connector
            .tasks
            .iter()
            .filter(|t| !only_failed || matches!(t.state, crate::connect::TaskState::Failed));

        let tasks: Vec<TaskStatus> = if include_tasks {
            task_iter
                .map(|t| TaskStatus {
                    id: t.id.task,
                    state: format!("{:?}", t.state).to_uppercase(),
                    worker_id: t.worker_id.clone(),
                    trace: t.trace.clone(),
                })
                .collect()
        } else {
            Vec::new()
        };

        Ok(RestartReport {
            status: HttpStatus::Accepted,
            state_info: ConnectorStateInfo {
                name: connector.name.clone(),
                connector_state: "RESTARTING".into(),
                tasks,
                connector_type: format!("{:?}", connector.connector_type).to_uppercase(),
            },
        })
    }

    // ── pause / resume / delete ─────────────────────────────────────────────

    pub fn pause_connector(&mut self, name: &str) -> Result<(), AdminError> {
        self.cluster
            .pause_connector(name)
            .map_err(|_| AdminError::NotFound(name.into()))
    }

    pub fn resume_connector(&mut self, name: &str) -> Result<(), AdminError> {
        self.cluster
            .resume_connector(name)
            .map_err(|_| AdminError::NotFound(name.into()))
    }

    pub fn delete_connector(&mut self, name: &str) -> Result<(), AdminError> {
        self.cluster
            .delete_connector(name)
            .map_err(|_| AdminError::NotFound(name.into()))
    }

    // ── status ──────────────────────────────────────────────────────────────

    /// GET /connectors/{c}/status. Returns `None` when unknown — the
    /// HTTP layer maps that to 404 (upstream throws
    /// `NotFoundException`).
    pub fn connector_status(&self, name: &str) -> Option<ConnectorStatus> {
        let connector = self.cluster.get_connector(name).ok()?;
        Some(ConnectorStatus {
            name: connector.name.clone(),
            connector_state: format!("{:?}", connector.state).to_uppercase(),
            tasks: connector
                .tasks
                .iter()
                .map(|t| TaskStatus {
                    id: t.id.task,
                    state: format!("{:?}", t.state).to_uppercase(),
                    worker_id: t.worker_id.clone(),
                    trace: t.trace.clone(),
                })
                .collect(),
            connector_type: format!("{:?}", connector.connector_type).to_uppercase(),
        })
    }

    // ── validate ────────────────────────────────────────────────────────────

    /// POST /connector-plugins/{class}/config/validate → 200 OK with
    /// `ConfigInfos { name, error_count, configs }`.
    pub fn validate_config(
        &self,
        plugin_class: &str,
        config: &HashMap<String, String>,
    ) -> ValidationResult {
        let mut configs = Vec::new();
        let mut error_count = 0i32;

        // connector.class is required upstream.
        let class_value = config.get("connector.class").cloned();
        let class_errors: Vec<String> = if class_value.is_none() {
            error_count += 1;
            vec!["Missing required configuration \"connector.class\" which has no default value.".into()]
        } else {
            Vec::new()
        };
        configs.push(ConfigInfo {
            name: "connector.class".into(),
            value: class_value,
            recommended_values: Vec::new(),
            validation_errors: class_errors,
            visible: true,
        });

        // tasks.max — recommended default 1; not required, no error
        // when absent.
        configs.push(ConfigInfo {
            name: "tasks.max".into(),
            value: config.get("tasks.max").cloned(),
            recommended_values: vec!["1".into()],
            validation_errors: Vec::new(),
            visible: true,
        });

        ValidationResult {
            status: HttpStatus::Ok,
            name: plugin_class.to_string(),
            error_count,
            configs,
        }
    }

    // ── server info ─────────────────────────────────────────────────────────

    /// GET / → 200 OK with `ServerInfo`. Mirrors upstream
    /// `RootResource.serverInfo()` returning `new
    /// ServerInfo(herder.kafkaClusterId())`.
    pub fn server_info(&self) -> ServerInfo {
        ServerInfo {
            status: HttpStatus::Ok,
            version: self.version.clone(),
            commit: self.commit.clone(),
            kafka_cluster_id: self.kafka_cluster_id.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(name: &str) -> HashMap<String, String> {
        let mut c = HashMap::new();
        c.insert(
            "connector.class".into(),
            "cave.connect.JdbcSourceConnector".into(),
        );
        c.insert("tasks.max".into(), "1".into());
        c.insert("name".into(), name.into());
        c
    }

    #[test]
    fn http_status_codes_match_upstream() {
        assert_eq!(HttpStatus::Ok.code(), 200);
        assert_eq!(HttpStatus::Created.code(), 201);
        assert_eq!(HttpStatus::Accepted.code(), 202);
        assert_eq!(HttpStatus::NoContent.code(), 204);
        assert_eq!(HttpStatus::BadRequest.code(), 400);
        assert_eq!(HttpStatus::NotFound.code(), 404);
        assert_eq!(HttpStatus::Conflict.code(), 409);
    }

    #[test]
    fn create_then_status_round_trip() {
        let mut a = ConnectRestAdmin::new();
        a.create_connector(Some("c".into()), cfg("c")).unwrap();
        let s = a.connector_status("c").unwrap();
        assert_eq!(s.name, "c");
        assert_eq!(s.connector_state, "RUNNING");
    }

    #[test]
    fn validate_config_zero_errors_when_class_present() {
        let a = ConnectRestAdmin::new();
        let r = a.validate_config("X", &cfg("y"));
        assert_eq!(r.error_count, 0);
    }
}

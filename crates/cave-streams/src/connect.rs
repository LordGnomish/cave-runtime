//! Connect API — source and sink connector framework.
//!
//! Provides a trait-based extensible framework for bridging external systems
//! to cave-streams topics.  Ships with an in-process connector registry that
//! manages connector lifecycle.

use crate::error::{StreamError, StreamResult};
use crate::models::{ConnectorConfig, ConnectorDirection, ConnectorStatus};
use crate::storage::StreamStorage;
use std::collections::HashMap;

// ─── Connector traits ─────────────────────────────────────────────────────────

/// A source connector reads data from an external system and produces it into
/// one or more cave-streams topics.
pub trait SourceConnector: Send + Sync {
    fn name(&self) -> &str;
    /// Called once when the connector is started.
    fn start(&mut self, config: &HashMap<String, String>) -> StreamResult<()>;
    /// Poll for the next batch of records to produce (non-blocking).
    fn poll(&mut self) -> StreamResult<Vec<ConnectorRecord>>;
    fn stop(&mut self) -> StreamResult<()>;
    fn status(&self) -> ConnectorStatus;
}

/// A sink connector reads data from cave-streams topics and writes it to an
/// external system.
pub trait SinkConnector: Send + Sync {
    fn name(&self) -> &str;
    fn start(&mut self, config: &HashMap<String, String>) -> StreamResult<()>;
    /// Deliver a batch of records from a topic to the external system.
    fn put(&mut self, records: Vec<ConnectorRecord>) -> StreamResult<()>;
    fn stop(&mut self) -> StreamResult<()>;
    fn status(&self) -> ConnectorStatus;
}

/// A record produced/consumed by a connector.
#[derive(Debug, Clone)]
pub struct ConnectorRecord {
    pub topic: String,
    pub partition: Option<u32>,
    pub key: Option<Vec<u8>>,
    pub value: Vec<u8>,
    pub headers: Vec<(String, Vec<u8>)>,
    pub timestamp_ms: Option<i64>,
}

// ─── Connector registry ───────────────────────────────────────────────────────

/// Manages connector configs in storage and exposes CRUD operations.
pub struct ConnectorRegistry<S: StreamStorage> {
    storage: S,
}

impl<S: StreamStorage> ConnectorRegistry<S> {
    pub fn new(storage: S) -> Self {
        Self { storage }
    }

    /// Register a new connector.
    pub fn create(&self, cfg: ConnectorConfig) -> StreamResult<ConnectorConfig> {
        validate_connector_config(&cfg)?;
        self.storage.create_connector(cfg.clone())?;
        Ok(cfg)
    }

    pub fn get(&self, name: &str) -> StreamResult<ConnectorConfig> {
        self.storage
            .get_connector(name)?
            .ok_or_else(|| StreamError::ConnectorNotFound(name.into()))
    }

    pub fn list(&self) -> StreamResult<Vec<ConnectorConfig>> {
        self.storage.list_connectors()
    }

    /// Pause a running connector.
    pub fn pause(&self, name: &str) -> StreamResult<ConnectorConfig> {
        let mut cfg = self.get(name)?;
        if cfg.status == ConnectorStatus::Running {
            cfg.status = ConnectorStatus::Paused;
            self.storage.update_connector(cfg.clone())?;
        }
        Ok(cfg)
    }

    /// Resume a paused connector.
    pub fn resume(&self, name: &str) -> StreamResult<ConnectorConfig> {
        let mut cfg = self.get(name)?;
        if cfg.status == ConnectorStatus::Paused {
            cfg.status = ConnectorStatus::Running;
            self.storage.update_connector(cfg.clone())?;
        }
        Ok(cfg)
    }

    /// Stop and delete a connector.
    pub fn delete(&self, name: &str) -> StreamResult<()> {
        self.storage.delete_connector(name)
    }

    /// Update connector config (causes a restart in a real implementation).
    pub fn update_config(
        &self,
        name: &str,
        new_config: HashMap<String, String>,
    ) -> StreamResult<ConnectorConfig> {
        let mut cfg = self.get(name)?;
        cfg.config.extend(new_config);
        self.storage.update_connector(cfg.clone())?;
        Ok(cfg)
    }
}

// ─── Built-in connector stubs ─────────────────────────────────────────────────

/// A no-op source connector (useful for testing the framework).
pub struct NoOpSourceConnector {
    name: String,
    status: ConnectorStatus,
}

impl NoOpSourceConnector {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status: ConnectorStatus::Stopped,
        }
    }
}

impl SourceConnector for NoOpSourceConnector {
    fn name(&self) -> &str {
        &self.name
    }

    fn start(&mut self, _config: &HashMap<String, String>) -> StreamResult<()> {
        self.status = ConnectorStatus::Running;
        Ok(())
    }

    fn poll(&mut self) -> StreamResult<Vec<ConnectorRecord>> {
        Ok(Vec::new())
    }

    fn stop(&mut self) -> StreamResult<()> {
        self.status = ConnectorStatus::Stopped;
        Ok(())
    }

    fn status(&self) -> ConnectorStatus {
        self.status.clone()
    }
}

/// A no-op sink connector (useful for testing the framework).
pub struct NoOpSinkConnector {
    name: String,
    status: ConnectorStatus,
    pub received: Vec<ConnectorRecord>,
}

impl NoOpSinkConnector {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status: ConnectorStatus::Stopped,
            received: Vec::new(),
        }
    }
}

impl SinkConnector for NoOpSinkConnector {
    fn name(&self) -> &str {
        &self.name
    }

    fn start(&mut self, _config: &HashMap<String, String>) -> StreamResult<()> {
        self.status = ConnectorStatus::Running;
        Ok(())
    }

    fn put(&mut self, records: Vec<ConnectorRecord>) -> StreamResult<()> {
        self.received.extend(records);
        Ok(())
    }

    fn stop(&mut self) -> StreamResult<()> {
        self.status = ConnectorStatus::Stopped;
        Ok(())
    }

    fn status(&self) -> ConnectorStatus {
        self.status.clone()
    }
}

// ─── Validation ───────────────────────────────────────────────────────────────

fn validate_connector_config(cfg: &ConnectorConfig) -> StreamResult<()> {
    if cfg.name.is_empty() {
        return Err(StreamError::Validation(
            "Connector name must not be empty".into(),
        ));
    }
    if cfg.connector_class.is_empty() {
        return Err(StreamError::Validation(
            "connector_class must not be empty".into(),
        ));
    }
    if cfg.topics.is_empty() {
        return Err(StreamError::Validation(
            "Connector must specify at least one topic".into(),
        ));
    }
    Ok(())
}

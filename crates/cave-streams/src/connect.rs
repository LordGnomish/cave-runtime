//! Kafka Connect API — connectors, tasks, transforms, and status.

use crate::error::{StreamsError, StreamsResult};
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// ── Connector types ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum ConnectorType {
    Source,
    Sink,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum ConnectorState {
    Unassigned,
    Running,
    Paused,
    Failed,
    Restarting,
    Destroyed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum TaskState {
    Unassigned,
    Running,
    Paused,
    Failed,
}

// ── Task ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: TaskId,
    pub config: HashMap<String, String>,
    pub state: TaskState,
    pub trace: Option<String>,
    pub worker_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TaskId {
    pub connector: String,
    pub task: usize,
}

// ── Transform ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transform {
    pub name: String,
    pub transform_type: String,
    pub config: HashMap<String, String>,
}

// ── Connector ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Connector {
    pub name: String,
    pub connector_type: ConnectorType,
    pub config: HashMap<String, String>,
    pub state: ConnectorState,
    pub tasks: Vec<Task>,
    pub transforms: Vec<Transform>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Connector {
    pub fn new(name: String, config: HashMap<String, String>) -> Self {
        let connector_type = match config.get("connector.class").map(|s| s.as_str()) {
            Some(cls) if cls.to_lowercase().contains("source") => ConnectorType::Source,
            Some(cls) if cls.to_lowercase().contains("sink") => ConnectorType::Sink,
            _ => ConnectorType::Unknown,
        };
        let tasks_max = config
            .get("tasks.max")
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(1);

        let tasks = (0..tasks_max)
            .map(|i| Task {
                id: TaskId { connector: name.clone(), task: i },
                config: config.clone(),
                state: TaskState::Unassigned,
                trace: None,
                worker_id: "worker-1".into(),
            })
            .collect();

        let transforms = Self::parse_transforms(&config);

        Self {
            name,
            connector_type,
            config,
            state: ConnectorState::Unassigned,
            tasks,
            transforms,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn parse_transforms(config: &HashMap<String, String>) -> Vec<Transform> {
        let names_str = config.get("transforms").cloned().unwrap_or_default();
        if names_str.is_empty() {
            return vec![];
        }
        names_str
            .split(',')
            .map(|name| {
                let name = name.trim().to_string();
                let transform_type = config
                    .get(&format!("transforms.{name}.type"))
                    .cloned()
                    .unwrap_or_else(|| "unknown".into());
                let prefix = format!("transforms.{name}.");
                let transform_config: HashMap<String, String> = config
                    .iter()
                    .filter(|(k, _)| k.starts_with(&prefix))
                    .map(|(k, v)| (k[prefix.len()..].to_string(), v.clone()))
                    .collect();
                Transform { name, transform_type, config: transform_config }
            })
            .collect()
    }

    pub fn update_config(&mut self, config: HashMap<String, String>) {
        self.config = config;
        self.transforms = Self::parse_transforms(&self.config);
        self.updated_at = Utc::now();
    }

    pub fn start(&mut self) {
        self.state = ConnectorState::Running;
        for task in &mut self.tasks {
            task.state = TaskState::Running;
        }
        self.updated_at = Utc::now();
    }

    pub fn pause(&mut self) {
        self.state = ConnectorState::Paused;
        for task in &mut self.tasks {
            task.state = TaskState::Paused;
        }
        self.updated_at = Utc::now();
    }

    pub fn resume(&mut self) {
        if self.state == ConnectorState::Paused {
            self.start();
        }
    }

    pub fn fail(&mut self, trace: Option<String>) {
        self.state = ConnectorState::Failed;
        self.updated_at = Utc::now();
        if let Some(t) = trace {
            for task in &mut self.tasks {
                task.state = TaskState::Failed;
                task.trace = Some(t.clone());
            }
        }
    }

    pub fn restart(&mut self) {
        self.state = ConnectorState::Restarting;
        self.updated_at = Utc::now();
    }
}

// ── Connect cluster ───────────────────────────────────────────────────────────

pub struct ConnectCluster {
    connectors: DashMap<String, Connector>,
    /// Registered connector plugins (class → description)
    plugins: DashMap<String, String>,
}

impl ConnectCluster {
    pub fn new() -> Self {
        let cluster = Self {
            connectors: DashMap::new(),
            plugins: DashMap::new(),
        };
        // Register built-in plugins
        cluster.plugins.insert(
            "org.apache.kafka.connect.file.FileStreamSourceConnector".into(),
            "File Stream Source Connector".into(),
        );
        cluster.plugins.insert(
            "org.apache.kafka.connect.file.FileStreamSinkConnector".into(),
            "File Stream Sink Connector".into(),
        );
        cluster.plugins.insert(
            "cave.connect.JdbcSourceConnector".into(),
            "JDBC Source Connector".into(),
        );
        cluster.plugins.insert(
            "cave.connect.JdbcSinkConnector".into(),
            "JDBC Sink Connector".into(),
        );
        cluster.plugins.insert(
            "cave.connect.S3SinkConnector".into(),
            "S3 Sink Connector".into(),
        );
        cluster.plugins.insert(
            "cave.connect.HttpSourceConnector".into(),
            "HTTP Source Connector".into(),
        );
        cluster
    }

    pub fn create_connector(
        &self,
        name: String,
        config: HashMap<String, String>,
    ) -> StreamsResult<Connector> {
        if self.connectors.contains_key(&name) {
            return Err(StreamsError::ConnectorAlreadyExists(name));
        }
        let mut connector = Connector::new(name.clone(), config);
        connector.start();
        let result = connector.clone();
        self.connectors.insert(name, connector);
        Ok(result)
    }

    pub fn get_connector(&self, name: &str) -> StreamsResult<Connector> {
        self.connectors
            .get(name)
            .map(|c| c.clone())
            .ok_or_else(|| StreamsError::ConnectorNotFound(name.into()))
    }

    pub fn update_connector_config(
        &self,
        name: &str,
        config: HashMap<String, String>,
    ) -> StreamsResult<Connector> {
        let mut conn = self
            .connectors
            .get_mut(name)
            .ok_or_else(|| StreamsError::ConnectorNotFound(name.into()))?;
        conn.update_config(config);
        Ok(conn.clone())
    }

    pub fn delete_connector(&self, name: &str) -> StreamsResult<()> {
        self.connectors
            .remove(name)
            .ok_or_else(|| StreamsError::ConnectorNotFound(name.into()))?;
        Ok(())
    }

    pub fn list_connectors(&self) -> Vec<String> {
        self.connectors.iter().map(|e| e.key().clone()).collect()
    }

    pub fn pause_connector(&self, name: &str) -> StreamsResult<()> {
        self.connectors
            .get_mut(name)
            .ok_or_else(|| StreamsError::ConnectorNotFound(name.into()))?
            .pause();
        Ok(())
    }

    pub fn resume_connector(&self, name: &str) -> StreamsResult<()> {
        self.connectors
            .get_mut(name)
            .ok_or_else(|| StreamsError::ConnectorNotFound(name.into()))?
            .resume();
        Ok(())
    }

    pub fn restart_connector(&self, name: &str) -> StreamsResult<()> {
        let mut conn = self
            .connectors
            .get_mut(name)
            .ok_or_else(|| StreamsError::ConnectorNotFound(name.into()))?;
        conn.restart();
        // After brief restarting period, start running
        conn.start();
        Ok(())
    }

    pub fn get_tasks(&self, connector: &str) -> StreamsResult<Vec<Task>> {
        Ok(self
            .connectors
            .get(connector)
            .ok_or_else(|| StreamsError::ConnectorNotFound(connector.into()))?
            .tasks
            .clone())
    }

    pub fn restart_task(&self, connector: &str, task_id: usize) -> StreamsResult<()> {
        let mut conn = self
            .connectors
            .get_mut(connector)
            .ok_or_else(|| StreamsError::ConnectorNotFound(connector.into()))?;
        let task = conn.tasks.get_mut(task_id).ok_or_else(|| {
            StreamsError::Internal(format!("task {task_id} not found in {connector}"))
        })?;
        task.state = TaskState::Running;
        task.trace = None;
        Ok(())
    }

    pub fn list_plugins(&self) -> Vec<PluginInfo> {
        self.plugins
            .iter()
            .map(|e| PluginInfo {
                class: e.key().clone(),
                plugin_type: if e.key().to_lowercase().contains("source") {
                    "source".into()
                } else {
                    "sink".into()
                },
                version: "1.0.0".into(),
            })
            .collect()
    }

    pub fn validate_config(
        &self,
        connector_class: &str,
        config: &HashMap<String, String>,
    ) -> ConfigValidation {
        let errors: Vec<String> = if !config.contains_key("connector.class") {
            vec!["connector.class is required".into()]
        } else {
            vec![]
        };
        ConfigValidation {
            name: connector_class.to_string(),
            error_count: errors.len() as i32,
            groups: vec![],
            configs: errors
                .iter()
                .map(|e| ConfigEntry {
                    name: "connector.class".into(),
                    value: config.get("connector.class").cloned(),
                    recommended_values: vec![],
                    errors: vec![e.clone()],
                    visible: true,
                })
                .collect(),
        }
    }
}

// ── DTOs ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct PluginInfo {
    pub class: String,
    #[serde(rename = "type")]
    pub plugin_type: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConfigValidation {
    pub name: String,
    pub error_count: i32,
    pub groups: Vec<String>,
    pub configs: Vec<ConfigEntry>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConfigEntry {
    pub name: String,
    pub value: Option<String>,
    pub recommended_values: Vec<String>,
    pub errors: Vec<String>,
    pub visible: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cluster() -> ConnectCluster {
        ConnectCluster::new()
    }

    fn jdbc_source_config() -> HashMap<String, String> {
        let mut c = HashMap::new();
        c.insert("connector.class".into(), "cave.connect.JdbcSourceConnector".into());
        c.insert("tasks.max".into(), "2".into());
        c.insert("connection.url".into(), "jdbc:postgresql://localhost/mydb".into());
        c.insert("topic.prefix".into(), "db-".into());
        c
    }

    #[test]
    fn create_and_list_connectors() {
        let c = cluster();
        c.create_connector("jdbc-source".into(), jdbc_source_config()).unwrap();
        let names = c.list_connectors();
        assert!(names.contains(&"jdbc-source".to_string()));
    }

    #[test]
    fn connector_starts_running() {
        let c = cluster();
        let conn = c.create_connector("my-conn".into(), jdbc_source_config()).unwrap();
        assert_eq!(conn.state, ConnectorState::Running);
        assert!(conn.tasks.iter().all(|t| t.state == TaskState::Running));
    }

    #[test]
    fn pause_and_resume() {
        let c = cluster();
        c.create_connector("pausable".into(), jdbc_source_config()).unwrap();
        c.pause_connector("pausable").unwrap();
        assert_eq!(c.get_connector("pausable").unwrap().state, ConnectorState::Paused);
        c.resume_connector("pausable").unwrap();
        assert_eq!(c.get_connector("pausable").unwrap().state, ConnectorState::Running);
    }

    #[test]
    fn duplicate_connector_fails() {
        let c = cluster();
        c.create_connector("dup".into(), jdbc_source_config()).unwrap();
        assert!(matches!(
            c.create_connector("dup".into(), jdbc_source_config()),
            Err(StreamsError::ConnectorAlreadyExists(_))
        ));
    }

    #[test]
    fn tasks_created_from_config() {
        let c = cluster();
        let conn = c.create_connector("multi-task".into(), jdbc_source_config()).unwrap();
        assert_eq!(conn.tasks.len(), 2);
    }

    #[test]
    fn list_plugins_non_empty() {
        let c = cluster();
        assert!(!c.list_plugins().is_empty());
    }
}

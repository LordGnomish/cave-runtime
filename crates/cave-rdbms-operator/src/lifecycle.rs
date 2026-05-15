// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use crate::error::{PgError, PgResult};
use crate::types::{InstanceState, PgInstance};
use chrono::Utc;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use uuid::Uuid;

pub struct InstanceManager {
    instances: Arc<RwLock<HashMap<String, PgInstance>>>,
}

impl Default for InstanceManager {
    fn default() -> Self {
        Self::new()
    }
}

impl InstanceManager {
    pub fn new() -> Self {
        InstanceManager {
            instances: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn create_instance(
        &self,
        name: &str,
        version: &str,
        host: &str,
        port: u16,
        database: &str,
        username: &str,
    ) -> PgResult<PgInstance> {
        let mut instances = self.instances.write().unwrap();

        // Check for duplicate name
        if instances.values().any(|i| i.name == name) {
            return Err(PgError::InstanceExists(name.to_string()));
        }

        let id = Uuid::new_v4().to_string();
        let now = Utc::now();
        let connection_string = format!(
            "postgres://{}@{}:{}/{}",
            username, host, port, database
        );

        let instance = PgInstance {
            id: id.clone(),
            name: name.to_string(),
            version: version.to_string(),
            state: InstanceState::Creating,
            connection_string,
            host: host.to_string(),
            port,
            database: database.to_string(),
            username: username.to_string(),
            max_connections: 100,
            shared_buffers_mb: 128,
            created_at: now,
            updated_at: now,
            labels: HashMap::new(),
            is_primary: true,
            replication_lag_bytes: None,
        };

        instances.insert(id, instance.clone());
        Ok(instance)
    }

    pub fn get_instance(&self, id: &str) -> PgResult<PgInstance> {
        let instances = self.instances.read().unwrap();
        instances
            .get(id)
            .cloned()
            .ok_or_else(|| PgError::InstanceNotFound(id.to_string()))
    }

    pub fn get_instance_by_name(&self, name: &str) -> PgResult<PgInstance> {
        let instances = self.instances.read().unwrap();
        instances
            .values()
            .find(|i| i.name == name)
            .cloned()
            .ok_or_else(|| PgError::InstanceNotFound(name.to_string()))
    }

    pub fn list_instances(&self) -> Vec<PgInstance> {
        let instances = self.instances.read().unwrap();
        instances.values().cloned().collect()
    }

    pub fn start_instance(&self, id: &str) -> PgResult<()> {
        let mut instances = self.instances.write().unwrap();
        let instance = instances
            .get_mut(id)
            .ok_or_else(|| PgError::InstanceNotFound(id.to_string()))?;
        instance.state = InstanceState::Running;
        instance.updated_at = Utc::now();
        Ok(())
    }

    pub fn stop_instance(&self, id: &str) -> PgResult<()> {
        let mut instances = self.instances.write().unwrap();
        let instance = instances
            .get_mut(id)
            .ok_or_else(|| PgError::InstanceNotFound(id.to_string()))?;
        instance.state = InstanceState::Stopped;
        instance.updated_at = Utc::now();
        Ok(())
    }

    pub fn restart_instance(&self, id: &str) -> PgResult<()> {
        let mut instances = self.instances.write().unwrap();
        let instance = instances
            .get_mut(id)
            .ok_or_else(|| PgError::InstanceNotFound(id.to_string()))?;
        instance.state = InstanceState::Restarting;
        instance.updated_at = Utc::now();
        // Immediately transition to Running (in-memory, no actual restart)
        instance.state = InstanceState::Running;
        Ok(())
    }

    pub fn delete_instance(&self, id: &str) -> PgResult<()> {
        let mut instances = self.instances.write().unwrap();
        if instances.remove(id).is_none() {
            return Err(PgError::InstanceNotFound(id.to_string()));
        }
        Ok(())
    }

    pub fn mark_failed(&self, id: &str, _reason: &str) -> PgResult<()> {
        let mut instances = self.instances.write().unwrap();
        let instance = instances
            .get_mut(id)
            .ok_or_else(|| PgError::InstanceNotFound(id.to_string()))?;
        instance.state = InstanceState::Failed;
        instance.updated_at = Utc::now();
        Ok(())
    }

    pub fn promote(&self, id: &str) -> PgResult<()> {
        let mut instances = self.instances.write().unwrap();
        let instance = instances
            .get_mut(id)
            .ok_or_else(|| PgError::InstanceNotFound(id.to_string()))?;
        instance.state = InstanceState::Promoting;
        instance.updated_at = Utc::now();
        instance.state = InstanceState::Running;
        instance.is_primary = true;
        instance.replication_lag_bytes = None;
        Ok(())
    }

    pub fn update_labels(&self, id: &str, labels: HashMap<String, String>) -> PgResult<()> {
        let mut instances = self.instances.write().unwrap();
        let instance = instances
            .get_mut(id)
            .ok_or_else(|| PgError::InstanceNotFound(id.to_string()))?;
        instance.labels = labels;
        instance.updated_at = Utc::now();
        Ok(())
    }

    pub fn connection_string(&self, id: &str) -> PgResult<String> {
        let instance = self.get_instance(id)?;
        Ok(instance.connection_string)
    }
}

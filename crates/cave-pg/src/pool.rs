use crate::error::{PgError, PgResult};
use crate::types::{PoolConfig, PoolMode};
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use uuid::Uuid;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct PoolStats {
    pub pool_name: String,
    pub total_connections: u32,
    pub active_connections: u32,
    pub idle_connections: u32,
    pub waiting_clients: u32,
    pub max_wait_ms: u64,
}

pub struct ConnectionPool {
    configs: Arc<RwLock<HashMap<String, PoolConfig>>>,
    stats: Arc<RwLock<HashMap<String, PoolStats>>>,
}

impl Default for ConnectionPool {
    fn default() -> Self {
        Self::new()
    }
}

impl ConnectionPool {
    pub fn new() -> Self {
        ConnectionPool {
            configs: Arc::new(RwLock::new(HashMap::new())),
            stats: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn create_pool(&self, config: PoolConfig) -> PgResult<()> {
        let mut configs = self.configs.write().unwrap();
        if configs.contains_key(&config.name) {
            return Err(PgError::ConfigError(format!(
                "pool already exists: {}",
                config.name
            )));
        }
        let stats = PoolStats {
            pool_name: config.name.clone(),
            total_connections: 0,
            active_connections: 0,
            idle_connections: 0,
            waiting_clients: 0,
            max_wait_ms: 0,
        };
        let mut stats_map = self.stats.write().unwrap();
        stats_map.insert(config.name.clone(), stats);
        configs.insert(config.name.clone(), config);
        Ok(())
    }

    pub fn remove_pool(&self, name: &str) -> PgResult<()> {
        let mut configs = self.configs.write().unwrap();
        if configs.remove(name).is_none() {
            return Err(PgError::ConfigError(format!("pool not found: {}", name)));
        }
        let mut stats = self.stats.write().unwrap();
        stats.remove(name);
        Ok(())
    }

    pub fn get_pool_config(&self, name: &str) -> PgResult<PoolConfig> {
        let configs = self.configs.read().unwrap();
        configs
            .get(name)
            .cloned()
            .ok_or_else(|| PgError::ConfigError(format!("pool not found: {}", name)))
    }

    pub fn list_pools(&self) -> Vec<PoolConfig> {
        let configs = self.configs.read().unwrap();
        configs.values().cloned().collect()
    }

    pub fn get_stats(&self, name: &str) -> PgResult<PoolStats> {
        let stats = self.stats.read().unwrap();
        stats
            .get(name)
            .cloned()
            .ok_or_else(|| PgError::ConfigError(format!("pool not found: {}", name)))
    }

    pub fn get_all_stats(&self) -> Vec<PoolStats> {
        let stats = self.stats.read().unwrap();
        stats.values().cloned().collect()
    }

    pub fn acquire(&self, pool_name: &str) -> PgResult<ConnectionHandle> {
        let configs = self.configs.read().unwrap();
        let config = configs
            .get(pool_name)
            .ok_or_else(|| PgError::ConfigError(format!("pool not found: {}", pool_name)))?;
        let mode = match config.mode {
            PoolMode::Session => PoolMode::Session,
            PoolMode::Transaction => PoolMode::Transaction,
            PoolMode::Statement => PoolMode::Statement,
        };
        drop(configs);

        // Update stats
        let mut stats = self.stats.write().unwrap();
        if let Some(s) = stats.get_mut(pool_name) {
            s.total_connections += 1;
            s.active_connections += 1;
        }

        Ok(ConnectionHandle {
            pool_name: pool_name.to_string(),
            connection_id: Uuid::new_v4().to_string(),
            acquired_at: Utc::now(),
            mode,
        })
    }

    pub fn release(&self, handle: ConnectionHandle) -> PgResult<()> {
        let mut stats = self.stats.write().unwrap();
        if let Some(s) = stats.get_mut(&handle.pool_name) {
            if s.active_connections > 0 {
                s.active_connections -= 1;
            }
            if s.total_connections > 0 {
                s.total_connections -= 1;
            }
        }
        Ok(())
    }

    pub fn mode_description(mode: &PoolMode) -> &'static str {
        match mode {
            PoolMode::Session => "session: connection held for full client session",
            PoolMode::Transaction => "transaction: connection returned after each transaction",
            PoolMode::Statement => "statement: connection returned after each statement",
        }
    }
}

pub struct ConnectionHandle {
    pub pool_name: String,
    pub connection_id: String,
    pub acquired_at: DateTime<Utc>,
    pub mode: PoolMode,
}

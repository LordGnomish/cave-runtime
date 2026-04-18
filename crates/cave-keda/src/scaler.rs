//! ScaledObject and ScaledJob stores.

use crate::error::{KedaError, KedaResult};
use crate::models::*;
use chrono::Utc;
use dashmap::DashMap;
use tracing::info;
use uuid::Uuid;

pub struct ScaledObjectStore {
    objects: DashMap<String, ScaledObject>,
    metrics: DashMap<String, Vec<MetricValue>>,
}

impl ScaledObjectStore {
    pub fn new() -> Self {
        Self { objects: DashMap::new(), metrics: DashMap::new() }
    }

    fn ns_key(ns: &str, name: &str) -> String { format!("{ns}/{name}") }

    pub fn create(&self, req: CreateScaledObjectRequest) -> KedaResult<ScaledObject> {
        let key = Self::ns_key(&req.namespace, &req.name);
        if self.objects.contains_key(&key) {
            return Err(KedaError::AlreadyExists(key));
        }
        let so = ScaledObject {
            id: Uuid::new_v4(),
            name: req.name.clone(),
            namespace: req.namespace.clone(),
            target_ref: req.target_ref,
            triggers: req.triggers,
            min_replica_count: req.min_replica_count,
            max_replica_count: req.max_replica_count.unwrap_or(100),
            polling_interval_secs: req.polling_interval_secs.unwrap_or(30),
            cooldown_period_secs: req.cooldown_period_secs.unwrap_or(300),
            status: ScaledObjectStatus::Unknown,
            current_replicas: req.min_replica_count.unwrap_or(0),
            desired_replicas: req.min_replica_count.unwrap_or(0),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        self.objects.insert(key, so.clone());
        info!(name = %req.name, namespace = %req.namespace, "ScaledObject created");
        Ok(so)
    }

    pub fn get(&self, ns: &str, name: &str) -> KedaResult<ScaledObject> {
        let key = Self::ns_key(ns, name);
        self.objects.get(&key).map(|r| r.clone()).ok_or_else(|| KedaError::ScaledObjectNotFound(key))
    }

    pub fn list(&self, ns: &str) -> Vec<ScaledObject> {
        self.objects.iter().filter(|r| r.value().namespace == ns).map(|r| r.value().clone()).collect()
    }

    pub fn delete(&self, ns: &str, name: &str) -> KedaResult<()> {
        let key = Self::ns_key(ns, name);
        self.objects.remove(&key).ok_or_else(|| KedaError::ScaledObjectNotFound(key))?;
        Ok(())
    }

    pub fn scale(&self, ns: &str, name: &str, desired: u32) -> KedaResult<ScaledObject> {
        let key = Self::ns_key(ns, name);
        let mut so = self.objects.get(&key).map(|r| r.clone()).ok_or_else(|| KedaError::ScaledObjectNotFound(key.clone()))?;
        so.desired_replicas = desired.clamp(
            so.min_replica_count.unwrap_or(0),
            so.max_replica_count,
        );
        so.current_replicas = so.desired_replicas;
        so.status = if so.current_replicas > 0 { ScaledObjectStatus::Active } else { ScaledObjectStatus::Inactive };
        so.updated_at = Utc::now();
        self.objects.insert(key, so.clone());
        Ok(so)
    }

    pub fn record_metrics(&self, ns: &str, name: &str, metrics: Vec<MetricValue>) {
        let key = Self::ns_key(ns, name);
        let mut stored = self.metrics.entry(key).or_default();
        stored.extend(metrics);
        let len = stored.len();
        if len > 100 {
            let excess = len - 100;
            stored.drain(0..excess);
        }
    }

    pub fn get_metrics(&self, ns: &str, name: &str) -> Vec<MetricValue> {
        let key = Self::ns_key(ns, name);
        self.metrics.get(&key).map(|r| r.clone()).unwrap_or_default()
    }
}

impl Default for ScaledObjectStore {
    fn default() -> Self { Self::new() }
}

pub struct ScaledJobStore {
    jobs: DashMap<String, ScaledJob>,
}

impl ScaledJobStore {
    pub fn new() -> Self {
        Self { jobs: DashMap::new() }
    }

    fn ns_key(ns: &str, name: &str) -> String { format!("{ns}/{name}") }

    pub fn create(&self, req: CreateScaledJobRequest) -> KedaResult<ScaledJob> {
        let key = Self::ns_key(&req.namespace, &req.name);
        if self.jobs.contains_key(&key) {
            return Err(KedaError::AlreadyExists(key));
        }
        let sj = ScaledJob {
            id: Uuid::new_v4(),
            name: req.name.clone(),
            namespace: req.namespace.clone(),
            job_template: req.job_template,
            triggers: req.triggers,
            max_replica_count: req.max_replica_count.unwrap_or(100),
            polling_interval_secs: req.polling_interval_secs.unwrap_or(30),
            status: ScaledJobStatus::Idle,
            active_jobs: 0,
            created_at: Utc::now(),
        };
        self.jobs.insert(key, sj.clone());
        Ok(sj)
    }

    pub fn get(&self, ns: &str, name: &str) -> KedaResult<ScaledJob> {
        let key = Self::ns_key(ns, name);
        self.jobs.get(&key).map(|r| r.clone()).ok_or_else(|| KedaError::ScaledJobNotFound(key))
    }

    pub fn list(&self, ns: &str) -> Vec<ScaledJob> {
        self.jobs.iter().filter(|r| r.value().namespace == ns).map(|r| r.value().clone()).collect()
    }

    pub fn delete(&self, ns: &str, name: &str) -> KedaResult<()> {
        let key = Self::ns_key(ns, name);
        self.jobs.remove(&key).ok_or_else(|| KedaError::ScaledJobNotFound(key))?;
        Ok(())
    }
}

impl Default for ScaledJobStore {
    fn default() -> Self { Self::new() }
}

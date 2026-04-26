//! Trigger type definitions and TriggerAuthentication store.

use crate::error::{KedaError, KedaResult};
use crate::models::{CreateTriggerAuthRequest, TriggerAuthentication};
use chrono::Utc;
use dashmap::DashMap;
use uuid::Uuid;

pub enum TriggerType {
    Kafka,
    Prometheus,
    Cron,
    Redis,
    Cpu,
    Memory,
    External,
    Aws,
    GcpPubSub,
    RabbitMq,
    Nats,
    ScaledJob,
    Http,
    Unknown,
}

impl TriggerType {
    pub fn as_str(&self) -> &'static str {
        match self {
            TriggerType::Kafka => "kafka",
            TriggerType::Prometheus => "prometheus",
            TriggerType::Cron => "cron",
            TriggerType::Redis => "redis",
            TriggerType::Cpu => "cpu",
            TriggerType::Memory => "memory",
            TriggerType::External => "external",
            TriggerType::Aws => "aws-sqs-queue",
            TriggerType::GcpPubSub => "gcp-pubsub",
            TriggerType::RabbitMq => "rabbitmq",
            TriggerType::Nats => "nats-jetstream",
            TriggerType::ScaledJob => "scaledjob",
            TriggerType::Http => "http",
            TriggerType::Unknown => "unknown",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "kafka" => TriggerType::Kafka,
            "prometheus" => TriggerType::Prometheus,
            "cron" => TriggerType::Cron,
            "redis" => TriggerType::Redis,
            "cpu" => TriggerType::Cpu,
            "memory" => TriggerType::Memory,
            "external" => TriggerType::External,
            "aws-sqs-queue" => TriggerType::Aws,
            "gcp-pubsub" => TriggerType::GcpPubSub,
            "rabbitmq" => TriggerType::RabbitMq,
            "nats-jetstream" => TriggerType::Nats,
            "scaledjob" => TriggerType::ScaledJob,
            "http" => TriggerType::Http,
            _ => TriggerType::Unknown,
        }
    }
}

pub struct TriggerAuthStore {
    auths: DashMap<String, TriggerAuthentication>,
}

impl TriggerAuthStore {
    pub fn new() -> Self {
        Self { auths: DashMap::new() }
    }

    fn ns_key(ns: &str, name: &str) -> String { format!("{ns}/{name}") }

    pub fn create(&self, req: CreateTriggerAuthRequest) -> KedaResult<TriggerAuthentication> {
        let key = Self::ns_key(&req.namespace, &req.name);
        if self.auths.contains_key(&key) {
            return Err(KedaError::AlreadyExists(key));
        }
        let auth = TriggerAuthentication {
            id: Uuid::new_v4(),
            name: req.name,
            namespace: req.namespace,
            spec: req.spec,
            created_at: Utc::now(),
        };
        self.auths.insert(key, auth.clone());
        Ok(auth)
    }

    pub fn get(&self, ns: &str, name: &str) -> KedaResult<TriggerAuthentication> {
        let key = Self::ns_key(ns, name);
        self.auths.get(&key).map(|r| r.clone()).ok_or_else(|| KedaError::TriggerAuthNotFound(key))
    }

    pub fn list(&self, ns: &str) -> Vec<TriggerAuthentication> {
        self.auths.iter().filter(|r| r.value().namespace == ns).map(|r| r.value().clone()).collect()
    }

    pub fn delete(&self, ns: &str, name: &str) -> KedaResult<()> {
        let key = Self::ns_key(ns, name);
        self.auths.remove(&key).ok_or_else(|| KedaError::TriggerAuthNotFound(key))?;
        Ok(())
    }
}

impl Default for TriggerAuthStore {
    fn default() -> Self { Self::new() }
}

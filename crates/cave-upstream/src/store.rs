// SPDX-License-Identifier: AGPL-3.0-or-later
//! In-memory store for cave-upstream.

use crate::models::{HealthCheck, UpstreamAlert, UpstreamService, UpstreamStats, UpstreamStatus};
use chrono::Utc;
use rand::Rng;
use std::collections::HashMap;
use std::sync::RwLock;
use uuid::Uuid;

#[derive(Default)]
pub struct UpstreamStore {
    pub services: RwLock<HashMap<Uuid, UpstreamService>>,
    pub health_checks: RwLock<HashMap<Uuid, Vec<HealthCheck>>>,
    pub alerts: RwLock<HashMap<Uuid, Vec<UpstreamAlert>>>,
}

impl UpstreamStore {
    pub fn new() -> Self {
        Self::default()
    }

    // ── Services CRUD ─────────────────────────────────────────────────────────

    pub fn insert_service(&self, service: UpstreamService) {
        self.services.write().unwrap().insert(service.id, service);
    }

    pub fn get_service(&self, id: Uuid) -> Option<UpstreamService> {
        self.services.read().unwrap().get(&id).cloned()
    }

    pub fn list_services(&self) -> Vec<UpstreamService> {
        let mut services: Vec<UpstreamService> = self.services.read().unwrap().values().cloned().collect();
        services.sort_by(|a, b| a.name.cmp(&b.name));
        services
    }

    pub fn update_service(&self, id: Uuid, updated: UpstreamService) -> Option<UpstreamService> {
        let mut services = self.services.write().unwrap();
        if services.contains_key(&id) {
            services.insert(id, updated.clone());
            Some(updated)
        } else {
            None
        }
    }

    pub fn delete_service(&self, id: Uuid) -> bool {
        self.services.write().unwrap().remove(&id).is_some()
    }

    // ── Health checks ─────────────────────────────────────────────────────────

    /// Simulate a health check for a service and store it.
    pub fn check_health_simulated(&self, service: &UpstreamService) -> HealthCheck {
        let mut rng = rand::thread_rng();
        // ~95% operational
        let operational = rng.gen_bool(0.95);
        let latency_ms: u64 = rng.gen_range(10..500);

        let status = if operational {
            if latency_ms > 400 { UpstreamStatus::Degraded } else { UpstreamStatus::Operational }
        } else {
            UpstreamStatus::Incident
        };

        let error = if !operational {
            Some("Simulated connection error".to_string())
        } else {
            None
        };

        let response_code: Option<u16> = if operational { Some(200) } else { Some(503) };

        let check = HealthCheck {
            upstream_id: service.id,
            checked_at: Utc::now(),
            latency_ms,
            status: status.clone(),
            error,
            response_code,
        };

        // Update last_checked_at on the service
        {
            let mut services = self.services.write().unwrap();
            if let Some(svc) = services.get_mut(&service.id) {
                svc.last_checked_at = Some(Utc::now());
                svc.status = status;
            }
        }

        // Store the check
        self.health_checks
            .write()
            .unwrap()
            .entry(service.id)
            .or_default()
            .push(check.clone());

        check
    }

    pub fn get_health_history(&self, service_id: Uuid) -> Vec<HealthCheck> {
        let checks = self.health_checks.read().unwrap();
        let mut history = checks.get(&service_id).cloned().unwrap_or_default();
        // Return last 50
        let len = history.len();
        if len > 50 {
            history = history[len - 50..].to_vec();
        }
        history.reverse();
        history
    }

    // ── Alerts ────────────────────────────────────────────────────────────────

    pub fn get_alerts(&self, service_id: Uuid) -> Vec<UpstreamAlert> {
        self.alerts.read().unwrap().get(&service_id).cloned().unwrap_or_default()
    }

    pub fn add_alert(&self, service_id: Uuid, alert: UpstreamAlert) {
        self.alerts
            .write()
            .unwrap()
            .entry(service_id)
            .or_default()
            .push(alert);
    }

    pub fn all_active_alerts(&self) -> Vec<UpstreamAlert> {
        self.alerts
            .read()
            .unwrap()
            .values()
            .flatten()
            .filter(|a| a.resolved_at.is_none())
            .cloned()
            .collect()
    }

    // ── Attention / Stats ─────────────────────────────────────────────────────

    pub fn services_needing_attention(&self) -> Vec<UpstreamService> {
        self.services
            .read()
            .unwrap()
            .values()
            .filter(|s| {
                matches!(
                    s.status,
                    UpstreamStatus::Deprecated
                        | UpstreamStatus::Eol
                        | UpstreamStatus::Degraded
                        | UpstreamStatus::Incident
                )
            })
            .cloned()
            .collect()
    }

    pub fn compute_stats(&self) -> UpstreamStats {
        let services = self.services.read().unwrap();
        let mut by_type: HashMap<String, u64> = HashMap::new();
        let mut operational = 0u64;
        let mut degraded = 0u64;
        let mut incidents = 0u64;
        let mut deprecated = 0u64;
        let mut eol = 0u64;

        for svc in services.values() {
            match svc.status {
                UpstreamStatus::Operational => operational += 1,
                UpstreamStatus::Degraded => degraded += 1,
                UpstreamStatus::Incident => incidents += 1,
                UpstreamStatus::Deprecated => deprecated += 1,
                UpstreamStatus::Eol => eol += 1,
            }
            let type_key = format!("{:?}", svc.upstream_type).to_lowercase();
            *by_type.entry(type_key).or_insert(0) += 1;
        }

        UpstreamStats {
            total: services.len() as u64,
            operational,
            degraded,
            incidents,
            deprecated,
            eol,
            by_type,
        }
    }

    // ── Seed demo data ────────────────────────────────────────────────────────

    pub fn seed_demo_data(&self) {
        use crate::models::{SupportTier, UpstreamType};

        let now = Utc::now();
        let demos: Vec<UpstreamService> = vec![
            UpstreamService {
                id: Uuid::new_v4(),
                name: "AWS S3".to_string(),
                description: "Amazon Simple Storage Service — object storage".to_string(),
                upstream_type: UpstreamType::CloudProvider,
                vendor: Some("Amazon Web Services".to_string()),
                version: None,
                status: UpstreamStatus::Operational,
                health_check_url: Some("https://health.aws.amazon.com/".to_string()),
                docs_url: Some("https://docs.aws.amazon.com/s3/".to_string()),
                license: Some("Proprietary".to_string()),
                support_tier: SupportTier::Enterprise,
                cost_per_month_usd: Some(120.0),
                deprecation_date: None,
                eol_date: None,
                alternatives: vec![],
                tags: vec!["storage".to_string(), "cloud".to_string()],
                last_checked_at: None,
                created_at: now,
                updated_at: now,
            },
            UpstreamService {
                id: Uuid::new_v4(),
                name: "GitHub API".to_string(),
                description: "GitHub REST and GraphQL APIs for repository management".to_string(),
                upstream_type: UpstreamType::ExternalApi,
                vendor: Some("GitHub / Microsoft".to_string()),
                version: Some("v3".to_string()),
                status: UpstreamStatus::Operational,
                health_check_url: Some("https://www.githubstatus.com/api/v2/status.json".to_string()),
                docs_url: Some("https://docs.github.com/en/rest".to_string()),
                license: Some("Proprietary".to_string()),
                support_tier: SupportTier::Commercial,
                cost_per_month_usd: Some(21.0),
                deprecation_date: None,
                eol_date: None,
                alternatives: vec!["GitLab API".to_string(), "Gitea API".to_string()],
                tags: vec!["vcs".to_string(), "api".to_string()],
                last_checked_at: None,
                created_at: now,
                updated_at: now,
            },
            UpstreamService {
                id: Uuid::new_v4(),
                name: "Stripe API".to_string(),
                description: "Payment processing and billing API".to_string(),
                upstream_type: UpstreamType::ExternalApi,
                vendor: Some("Stripe".to_string()),
                version: Some("2023-10-16".to_string()),
                status: UpstreamStatus::Operational,
                health_check_url: Some("https://status.stripe.com/api/v2/status.json".to_string()),
                docs_url: Some("https://stripe.com/docs/api".to_string()),
                license: Some("Proprietary".to_string()),
                support_tier: SupportTier::Commercial,
                cost_per_month_usd: None,
                deprecation_date: None,
                eol_date: None,
                alternatives: vec!["Braintree".to_string()],
                tags: vec!["payments".to_string(), "billing".to_string()],
                last_checked_at: None,
                created_at: now,
                updated_at: now,
            },
            UpstreamService {
                id: Uuid::new_v4(),
                name: "Datadog".to_string(),
                description: "Cloud-scale monitoring and observability platform".to_string(),
                upstream_type: UpstreamType::ManagedService,
                vendor: Some("Datadog".to_string()),
                version: None,
                status: UpstreamStatus::Operational,
                health_check_url: Some("https://status.datadoghq.com/".to_string()),
                docs_url: Some("https://docs.datadoghq.com/".to_string()),
                license: Some("Proprietary".to_string()),
                support_tier: SupportTier::Enterprise,
                cost_per_month_usd: Some(800.0),
                deprecation_date: None,
                eol_date: None,
                alternatives: vec!["Grafana Cloud".to_string(), "New Relic".to_string()],
                tags: vec!["observability".to_string(), "monitoring".to_string()],
                last_checked_at: None,
                created_at: now,
                updated_at: now,
            },
            UpstreamService {
                id: Uuid::new_v4(),
                name: "PagerDuty".to_string(),
                description: "On-call and incident management platform".to_string(),
                upstream_type: UpstreamType::ManagedService,
                vendor: Some("PagerDuty".to_string()),
                version: None,
                status: UpstreamStatus::Operational,
                health_check_url: Some("https://status.pagerduty.com/api/v2/status.json".to_string()),
                docs_url: Some("https://developer.pagerduty.com/".to_string()),
                license: Some("Proprietary".to_string()),
                support_tier: SupportTier::Commercial,
                cost_per_month_usd: Some(150.0),
                deprecation_date: None,
                eol_date: None,
                alternatives: vec!["Grafana OnCall".to_string(), "OpsGenie".to_string()],
                tags: vec!["incidents".to_string(), "on-call".to_string()],
                last_checked_at: None,
                created_at: now,
                updated_at: now,
            },
            UpstreamService {
                id: Uuid::new_v4(),
                name: "PostgreSQL OSS".to_string(),
                description: "Open source relational database".to_string(),
                upstream_type: UpstreamType::OpenSourceLib,
                vendor: None,
                version: Some("16.2".to_string()),
                status: UpstreamStatus::Operational,
                health_check_url: None,
                docs_url: Some("https://www.postgresql.org/docs/".to_string()),
                license: Some("PostgreSQL License".to_string()),
                support_tier: SupportTier::Community,
                cost_per_month_usd: None,
                deprecation_date: None,
                eol_date: None,
                alternatives: vec![],
                tags: vec!["database".to_string(), "sql".to_string()],
                last_checked_at: None,
                created_at: now,
                updated_at: now,
            },
            UpstreamService {
                id: Uuid::new_v4(),
                name: "Redis OSS".to_string(),
                description: "In-memory data structure store, cache and message broker".to_string(),
                upstream_type: UpstreamType::OpenSourceLib,
                vendor: None,
                version: Some("7.2".to_string()),
                status: UpstreamStatus::Operational,
                health_check_url: None,
                docs_url: Some("https://redis.io/docs/".to_string()),
                license: Some("BSD-3-Clause".to_string()),
                support_tier: SupportTier::Community,
                cost_per_month_usd: None,
                deprecation_date: None,
                eol_date: None,
                alternatives: vec!["Valkey".to_string(), "DragonflyDB".to_string()],
                tags: vec!["cache".to_string(), "database".to_string()],
                last_checked_at: None,
                created_at: now,
                updated_at: now,
            },
            UpstreamService {
                id: Uuid::new_v4(),
                name: "Kafka Confluent".to_string(),
                description: "Managed Apache Kafka event streaming platform".to_string(),
                upstream_type: UpstreamType::ManagedService,
                vendor: Some("Confluent".to_string()),
                version: Some("3.6".to_string()),
                status: UpstreamStatus::Operational,
                health_check_url: None,
                docs_url: Some("https://docs.confluent.io/".to_string()),
                license: Some("Proprietary".to_string()),
                support_tier: SupportTier::Enterprise,
                cost_per_month_usd: Some(500.0),
                deprecation_date: None,
                eol_date: None,
                alternatives: vec!["Redpanda".to_string(), "MSK".to_string()],
                tags: vec!["streaming".to_string(), "messaging".to_string()],
                last_checked_at: None,
                created_at: now,
                updated_at: now,
            },
            UpstreamService {
                id: Uuid::new_v4(),
                name: "HashiCorp Vault".to_string(),
                description: "Secrets management and data protection".to_string(),
                upstream_type: UpstreamType::OpenSourceLib,
                vendor: Some("HashiCorp".to_string()),
                version: Some("1.15".to_string()),
                status: UpstreamStatus::Operational,
                health_check_url: None,
                docs_url: Some("https://developer.hashicorp.com/vault/docs".to_string()),
                license: Some("BUSL-1.1".to_string()),
                support_tier: SupportTier::Commercial,
                cost_per_month_usd: None,
                deprecation_date: None,
                eol_date: None,
                alternatives: vec!["OpenBao".to_string(), "AWS Secrets Manager".to_string()],
                tags: vec!["secrets".to_string(), "security".to_string()],
                last_checked_at: None,
                created_at: now,
                updated_at: now,
            },
            UpstreamService {
                id: Uuid::new_v4(),
                name: "Kubernetes".to_string(),
                description: "Container orchestration platform".to_string(),
                upstream_type: UpstreamType::OpenSourceLib,
                vendor: None,
                version: Some("1.29".to_string()),
                status: UpstreamStatus::Operational,
                health_check_url: None,
                docs_url: Some("https://kubernetes.io/docs/".to_string()),
                license: Some("Apache-2.0".to_string()),
                support_tier: SupportTier::Community,
                cost_per_month_usd: None,
                deprecation_date: None,
                eol_date: None,
                alternatives: vec![],
                tags: vec!["orchestration".to_string(), "containers".to_string()],
                last_checked_at: None,
                created_at: now,
                updated_at: now,
            },
        ];

        for svc in demos {
            // Generate alerts for each service
            let alerts = crate::engine::generate_alerts(&svc);
            let svc_id = svc.id;
            self.insert_service(svc);
            for alert in alerts {
                self.add_alert(svc_id, alert);
            }
        }
    }
}

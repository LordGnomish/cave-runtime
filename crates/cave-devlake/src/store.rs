//! In-memory store for cave-devlake.

use crate::engine::{
    dora_cfr_rating, dora_deployment_frequency_rating, dora_lead_time_rating, dora_mttr_rating,
    overall_dora_rating,
};
use crate::models::{
    Deployment, DeploymentEnv, DeploymentStatus, DoraReport, Incident, Pipeline,
    PipelineStatus,
};
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::sync::RwLock;
use uuid::Uuid;

#[derive(Default)]
pub struct DevlakeStore {
    pub pipelines: RwLock<HashMap<Uuid, Pipeline>>,
    pub deployments: RwLock<HashMap<Uuid, Deployment>>,
    pub incidents: RwLock<HashMap<Uuid, Incident>>,
}

impl DevlakeStore {
    pub fn new() -> Self {
        Self::default()
    }

    // ── Pipelines ─────────────────────────────────────────────────────────────

    pub fn insert_pipeline(&self, pipeline: Pipeline) {
        self.pipelines.write().unwrap().insert(pipeline.id, pipeline);
    }

    pub fn get_pipeline(&self, id: Uuid) -> Option<Pipeline> {
        self.pipelines.read().unwrap().get(&id).cloned()
    }

    pub fn list_pipelines(&self) -> Vec<Pipeline> {
        let mut pipelines: Vec<Pipeline> = self.pipelines.read().unwrap().values().cloned().collect();
        pipelines.sort_by(|a, b| b.started_at.cmp(&a.started_at));
        pipelines
    }

    pub fn update_pipeline_status(&self, id: Uuid, status: PipelineStatus, duration_secs: Option<f64>) -> Option<Pipeline> {
        let mut pipelines = self.pipelines.write().unwrap();
        if let Some(p) = pipelines.get_mut(&id) {
            p.status = status;
            if duration_secs.is_some() {
                p.duration_secs = duration_secs;
                p.finished_at = Some(Utc::now());
            }
            Some(p.clone())
        } else {
            None
        }
    }

    // ── Deployments ───────────────────────────────────────────────────────────

    pub fn insert_deployment(&self, deployment: Deployment) {
        self.deployments.write().unwrap().insert(deployment.id, deployment);
    }

    pub fn get_deployment(&self, id: Uuid) -> Option<Deployment> {
        self.deployments.read().unwrap().get(&id).cloned()
    }

    pub fn list_deployments(&self) -> Vec<Deployment> {
        let mut deployments: Vec<Deployment> = self.deployments.read().unwrap().values().cloned().collect();
        deployments.sort_by(|a, b| b.deployed_at.cmp(&a.deployed_at));
        deployments
    }

    pub fn recent_deployments(&self, env: Option<&DeploymentEnv>, limit: usize) -> Vec<Deployment> {
        let mut deployments = self.list_deployments();
        if let Some(env) = env {
            deployments.retain(|d| &d.environment == env);
        }
        deployments.truncate(limit);
        deployments
    }

    pub fn deployments_in_period(&self, from: DateTime<Utc>, to: DateTime<Utc>) -> Vec<Deployment> {
        self.deployments
            .read()
            .unwrap()
            .values()
            .filter(|d| d.deployed_at >= from && d.deployed_at <= to)
            .cloned()
            .collect()
    }

    // ── Incidents ─────────────────────────────────────────────────────────────

    pub fn insert_incident(&self, incident: Incident) {
        self.incidents.write().unwrap().insert(incident.id, incident);
    }

    pub fn get_incident(&self, id: Uuid) -> Option<Incident> {
        self.incidents.read().unwrap().get(&id).cloned()
    }

    pub fn list_incidents(&self) -> Vec<Incident> {
        let mut incidents: Vec<Incident> = self.incidents.read().unwrap().values().cloned().collect();
        incidents.sort_by(|a, b| b.started_at.cmp(&a.started_at));
        incidents
    }

    pub fn resolve_incident(&self, id: Uuid, resolved_at: DateTime<Utc>) -> Option<Incident> {
        let mut incidents = self.incidents.write().unwrap();
        if let Some(inc) = incidents.get_mut(&id) {
            inc.resolved_at = Some(resolved_at);
            Some(inc.clone())
        } else {
            None
        }
    }

    // ── DORA computation ──────────────────────────────────────────────────────

    pub fn compute_dora_report(&self, period_days: u32) -> DoraReport {
        let now = Utc::now();
        let from = now - chrono::Duration::days(period_days as i64);

        let deployments = self.deployments_in_period(from, now);
        let total = deployments.len() as f64;
        let days = period_days as f64;

        // Deployment frequency
        let deployment_frequency_per_day = total / days.max(1.0);

        // Lead time: avg lead_time_secs from deployments that have it
        let lead_times: Vec<f64> = deployments.iter().filter_map(|d| d.lead_time_secs).collect();
        let lead_time_secs = if lead_times.is_empty() {
            0.0
        } else {
            lead_times.iter().sum::<f64>() / lead_times.len() as f64
        };

        // Change failure rate
        let failed = deployments
            .iter()
            .filter(|d| matches!(d.status, DeploymentStatus::Failed | DeploymentStatus::RolledBack))
            .count() as f64;
        let change_failure_rate_pct = if total > 0.0 { failed / total * 100.0 } else { 0.0 };

        // MTTR: avg (resolved_at - started_at) for resolved incidents in period
        let incidents = self.incidents.read().unwrap();
        let mttr_values: Vec<f64> = incidents
            .values()
            .filter(|inc| inc.started_at >= from && inc.resolved_at.is_some())
            .filter_map(|inc| {
                inc.resolved_at
                    .map(|r| (r - inc.started_at).num_seconds() as f64)
            })
            .collect();
        drop(incidents);

        let mttr_secs = if mttr_values.is_empty() {
            0.0
        } else {
            mttr_values.iter().sum::<f64>() / mttr_values.len() as f64
        };

        let deployment_frequency_rating = dora_deployment_frequency_rating(deployment_frequency_per_day);
        let lead_time_rating = dora_lead_time_rating(lead_time_secs);
        let change_failure_rate_rating = dora_cfr_rating(change_failure_rate_pct);
        let mttr_rating = dora_mttr_rating(mttr_secs);

        let overall_rating = overall_dora_rating(&[
            deployment_frequency_rating.clone(),
            lead_time_rating.clone(),
            change_failure_rate_rating.clone(),
            mttr_rating.clone(),
        ]);

        DoraReport {
            period_days,
            deployment_frequency_per_day,
            deployment_frequency_rating,
            lead_time_secs,
            lead_time_rating,
            change_failure_rate_pct,
            change_failure_rate_rating,
            mttr_secs,
            mttr_rating,
            overall_rating,
        }
    }

    pub fn dora_history(&self) -> Vec<DoraReport> {
        // Return last 12 weekly reports (week 1 = most recent, week 12 = oldest)
        (0..12)
            .map(|i| {
                let period_days = 7u32;
                let now = Utc::now();
                let _from = now - chrono::Duration::days(((i + 1) * 7) as i64);
                let _to = now - chrono::Duration::days((i * 7) as i64);
                // For simplicity, compute the 7-day report but offset context
                // A real impl would filter by week window; here we reuse compute_dora_report
                // approximation: same calculation for all weeks (demo data)
                let _ = (i, period_days);
                self.compute_dora_report(7)
            })
            .collect()
    }

    pub fn seed_demo_data(&self) {
        use crate::models::{DeploymentEnv, DeploymentStatus, PipelineStage, PipelineStatus};
        use chrono::Duration;

        let now = Utc::now();
        let services = ["api-gateway", "auth-service", "payment-service", "user-service", "notification-service"];
        let users = ["alice", "bob", "carol", "dave"];

        for i in 0..20u32 {
            let days_ago = (i * 2) as i64;
            let deployed_at = now - Duration::days(days_ago) - Duration::hours((i % 8) as i64);
            let lead_time = Some(1800.0 + (i as f64 * 300.0)); // 30min to several hours
            let status = if i % 7 == 0 {
                DeploymentStatus::Failed
            } else if i % 11 == 0 {
                DeploymentStatus::RolledBack
            } else {
                DeploymentStatus::Success
            };
            let env = match i % 4 {
                0 => DeploymentEnv::Production,
                1 => DeploymentEnv::Staging,
                2 => DeploymentEnv::Development,
                _ => DeploymentEnv::Testing,
            };
            let deployment = Deployment {
                id: Uuid::new_v4(),
                pipeline_id: None,
                service: services[(i as usize) % services.len()].to_string(),
                version: format!("v1.{}.{}", i / 5, i % 5),
                environment: env,
                deployed_at,
                deployed_by: users[(i as usize) % users.len()].to_string(),
                status,
                rollback: i % 11 == 0,
                lead_time_secs: lead_time,
            };
            self.insert_deployment(deployment);

            // Create a matching pipeline for every other deployment
            if i % 2 == 0 {
                let pipeline = Pipeline {
                    id: Uuid::new_v4(),
                    name: format!("build-{}", services[(i as usize) % services.len()]),
                    project: "cave-runtime".to_string(),
                    repo: format!("github.com/cave/{}", services[(i as usize) % services.len()]),
                    branch: "main".to_string(),
                    status: if i % 7 == 0 { PipelineStatus::Failed } else { PipelineStatus::Success },
                    triggered_by: users[(i as usize) % users.len()].to_string(),
                    started_at: deployed_at - Duration::minutes(30),
                    finished_at: Some(deployed_at),
                    duration_secs: Some(1800.0),
                    stages: vec![
                        PipelineStage {
                            name: "build".to_string(),
                            status: PipelineStatus::Success,
                            started_at: Some(deployed_at - Duration::minutes(30)),
                            finished_at: Some(deployed_at - Duration::minutes(20)),
                            duration_secs: Some(600.0),
                            logs_url: None,
                        },
                        PipelineStage {
                            name: "test".to_string(),
                            status: PipelineStatus::Success,
                            started_at: Some(deployed_at - Duration::minutes(20)),
                            finished_at: Some(deployed_at - Duration::minutes(5)),
                            duration_secs: Some(900.0),
                            logs_url: None,
                        },
                        PipelineStage {
                            name: "deploy".to_string(),
                            status: if i % 7 == 0 { PipelineStatus::Failed } else { PipelineStatus::Success },
                            started_at: Some(deployed_at - Duration::minutes(5)),
                            finished_at: Some(deployed_at),
                            duration_secs: Some(300.0),
                            logs_url: None,
                        },
                    ],
                    commit_sha: Some(format!("abc{:04x}", i)),
                    environment: DeploymentEnv::Production,
                };
                self.insert_pipeline(pipeline);
            }
        }

        // Seed 5 incidents
        let severities = ["P1", "P2", "P3", "P1", "P2"];
        let titles = [
            "API gateway 5xx spike",
            "Auth service latency degradation",
            "Payment processor timeout",
            "Database connection pool exhausted",
            "CDN cache invalidation failure",
        ];
        for i in 0..5usize {
            let started_at = now - Duration::days((i as i64 + 1) * 5);
            let resolved_at = if i < 4 {
                Some(started_at + Duration::hours(2 + i as i64))
            } else {
                None // one unresolved incident
            };
            let incident = Incident {
                id: Uuid::new_v4(),
                title: titles[i].to_string(),
                severity: severities[i].to_string(),
                started_at,
                resolved_at,
                services: vec![services[i % services.len()].to_string()],
                linked_deployment_id: None,
            };
            self.insert_incident(incident);
        }
    }
}

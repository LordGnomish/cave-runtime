// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! ScaledJob CRD — autoscale Job workloads with per-event Job creation.
//! upstream: kedacore/keda v2.x — apis/keda/v1alpha1/scaledjob_types.go

use std::time::Duration;

#[derive(Default, Debug, Clone)]
pub struct ScaledJob {
    pub tenant_id: String,
    pub max_replica_count: Option<i32>,
    pub polling_interval: Option<Duration>,
    pub successful_jobs_history_limit: Option<i32>,
    pub failed_jobs_history_limit: Option<i32>,
    pub scaling_strategy: ScalingStrategy,
    pub running_jobs: i32,
    pub successful_jobs: Vec<String>,
    pub failed_jobs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScalingStrategy {
    /// Default — desired = queue_length
    Default,
    /// Custom — desired = (queue_length - running_jobs) min(max_replicas)
    Custom,
    /// Accurate — desired = max(queue_length - pending_jobs, 0)
    Accurate,
}

impl Default for ScalingStrategy {
    fn default() -> Self {
        ScalingStrategy::Default
    }
}

impl ScaledJob {
    pub fn new(tenant_id: &str) -> Self {
        Self {
            tenant_id: tenant_id.to_string(),
            max_replica_count: Some(100),
            polling_interval: Some(Duration::from_secs(30)),
            successful_jobs_history_limit: Some(100),
            failed_jobs_history_limit: Some(100),
            scaling_strategy: ScalingStrategy::Default,
            running_jobs: 0,
            successful_jobs: Vec::new(),
            failed_jobs: Vec::new(),
        }
    }

    /// Compute jobs to spawn for a given queue length, honoring scaling strategy
    /// and max_replica_count ceiling.
    pub fn jobs_to_spawn(&self, queue_length: i64) -> i32 {
        let max = self.max_replica_count.unwrap_or(100);
        let desired = match self.scaling_strategy {
            ScalingStrategy::Default => queue_length as i32,
            ScalingStrategy::Custom => (queue_length as i32 - self.running_jobs).max(0),
            ScalingStrategy::Accurate => (queue_length as i32 - self.running_jobs).max(0),
        };
        desired.clamp(0, max)
    }

    /// Record a job's terminal outcome. Trims the history to the configured limit.
    pub fn record_outcome(&mut self, job_id: &str, success: bool) {
        let (vec, limit_field) = if success {
            (
                &mut self.successful_jobs,
                self.successful_jobs_history_limit,
            )
        } else {
            (&mut self.failed_jobs, self.failed_jobs_history_limit)
        };
        vec.push(job_id.to_string());
        let limit = limit_field.unwrap_or(100) as usize;
        if vec.len() > limit {
            let excess = vec.len() - limit;
            vec.drain(0..excess);
        }
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Knative Revision — immutable snapshot of code + config.
//! upstream: knative/serving v1.18.x — pkg/apis/serving/v1/revision_types.go

use crate::meta::{ObjectMeta, RevisionTemplateSpec, TrafficTarget, validate_template};

#[derive(Default, Debug, Clone)]
pub struct Revision {
    pub metadata: ObjectMeta,
    pub spec: RevisionSpec,
    pub status: RevisionStatus,
}

#[derive(Default, Debug, Clone)]
pub struct RevisionSpec {
    pub containerConcurrency: Option<i32>,
    pub timeoutSeconds: Option<i32>,
    pub template: RevisionTemplateSpec,
}

#[derive(Default, Debug, Clone)]
pub struct RevisionStatus {
    pub actualReplicas: Option<i32>,
    pub desiredReplicas: Option<i32>,
    pub traffic: Vec<TrafficTarget>,
    pub observed_generation: i64,
}

impl Revision {
    pub fn new(tenant_id: &str) -> Self {
        Self {
            metadata: ObjectMeta::with_creator(tenant_id),
            spec: RevisionSpec::default(),
            status: RevisionStatus::default(),
        }
    }

    /// Drop desired replica count to 0 (the cornerstone of scale-to-zero).
    pub fn scale_to_zero(&mut self) {
        self.status.desiredReplicas = Some(0);
    }

    pub fn name(&self) -> String {
        self.metadata.name.clone()
    }

    pub fn validate(&self) -> Result<(), String> {
        if let Some(c) = self.spec.containerConcurrency {
            if c < 0 {
                return Err("containerConcurrency must be >= 0".to_string());
            }
        }
        if let Some(t) = self.spec.timeoutSeconds {
            if t <= 0 {
                return Err("timeoutSeconds must be > 0".to_string());
            }
        }
        validate_template(&self.spec.template)?;
        Ok(())
    }

    /// Set desired replica count (called by the autoscaler).
    pub fn set_desired_replicas(&mut self, replicas: i32) {
        self.status.desiredReplicas = Some(replicas.max(0));
    }

    /// Mark actual replica count (called when the underlying Deployment reports status).
    pub fn set_actual_replicas(&mut self, replicas: i32) {
        self.status.actualReplicas = Some(replicas.max(0));
    }

    pub fn is_active(&self) -> bool {
        self.status.desiredReplicas.unwrap_or(0) > 0
    }
}

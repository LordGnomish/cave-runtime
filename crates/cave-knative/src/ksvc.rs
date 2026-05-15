// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Knative Service (ksvc) — top-level CRD wrapping Configuration + Route.
//! upstream: knative/serving v1.18.x — pkg/apis/serving/v1/service_types.go

use crate::meta::{
    validate_template, validate_traffic, ObjectMeta, RevisionTemplateSpec, TrafficTarget,
};

#[derive(Default, Debug, Clone)]
pub struct Ksvc {
    pub metadata: ObjectMeta,
    pub spec: ServiceSpec,
    pub status: ServiceStatus,
}

#[derive(Default, Debug, Clone)]
pub struct ServiceSpec {
    pub template: RevisionTemplateSpec,
    pub traffic: Vec<TrafficTarget>,
}

#[derive(Default, Debug, Clone)]
pub struct ServiceStatus {
    pub traffic: Vec<TrafficTarget>,
    pub latest_created_revision_name: Option<String>,
    pub latest_ready_revision_name: Option<String>,
    pub url: Option<String>,
    pub observed_generation: i64,
}

impl Ksvc {
    pub fn new(tenant_id: &str) -> Self {
        Self {
            metadata: ObjectMeta::with_creator(tenant_id),
            spec: ServiceSpec::default(),
            status: ServiceStatus::default(),
        }
    }

    /// Drop the service to scale-zero state: traffic targets stay but desired replicas
    /// are implicitly 0. Mirrors `kubectl scale ksvc/foo --replicas=0` semantics.
    pub fn scale_to_zero(&mut self) {
        for t in &mut self.status.traffic {
            t.percent = Some(0);
        }
    }

    pub fn name(&self) -> String {
        self.metadata.name.clone()
    }

    /// Validate that the service spec is internally consistent. If the spec.traffic is empty,
    /// the service is in "single revision, 100% latest" mode and is valid.
    pub fn validate(&self) -> Result<(), String> {
        validate_template(&self.spec.template)?;
        if !self.spec.traffic.is_empty() {
            validate_traffic(&self.spec.traffic)?;
        }
        Ok(())
    }
}

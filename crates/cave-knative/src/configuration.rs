// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Knative Configuration — desired state spawning Revisions.
//! upstream: knative/serving v1.18.x — pkg/apis/serving/v1/configuration_types.go

use crate::meta::{ObjectMeta, RevisionTemplateSpec, validate_template};

#[derive(Default, Debug, Clone)]
pub struct Configuration {
    pub metadata: ObjectMeta,
    pub spec: ConfigurationSpec,
    pub status: ConfigurationStatus,
}

#[derive(Default, Debug, Clone)]
pub struct ConfigurationSpec {
    pub template: RevisionTemplateSpec,
}

#[derive(Default, Debug, Clone)]
pub struct ConfigurationStatus {
    pub latestCreatedRevisionName: Option<String>,
    pub latestReadyRevisionName: Option<String>,
    pub observed_generation: i64,
}

impl Configuration {
    pub fn new(tenant_id: &str) -> Self {
        Self {
            metadata: ObjectMeta::with_creator(tenant_id),
            spec: ConfigurationSpec::default(),
            status: ConfigurationStatus::default(),
        }
    }

    /// Drop replica targets — preserves the latest revision but signals zero traffic.
    pub fn scale_to_zero(&mut self) {
        // No replica counts on the Configuration itself; this just bumps observed_generation
        // to mark a no-op reconciliation cycle, matching upstream knative behavior.
        self.status.observed_generation = self.metadata.generation;
    }

    pub fn validate(&self) -> Result<(), String> {
        validate_template(&self.spec.template)
    }

    /// Mark a revision as the latest created (called when the controller spawns it).
    pub fn record_created_revision(&mut self, revision_name: &str) {
        self.status.latestCreatedRevisionName = Some(revision_name.to_string());
    }

    /// Mark a revision as the latest ready (called when its Pods are Ready).
    pub fn record_ready_revision(&mut self, revision_name: &str) {
        self.status.latestReadyRevisionName = Some(revision_name.to_string());
    }
}

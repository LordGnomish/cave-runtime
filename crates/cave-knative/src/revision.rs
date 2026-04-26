//! Knative Revision — immutable snapshot of code + config.
//! upstream: knative/serving v1.18.x — pkg/apis/serving/v1/revision_types.go

use crate::meta::{ObjectMeta, RevisionTemplateSpec, TrafficTarget};

#[derive(Default)]
pub struct Revision {
    pub metadata: ObjectMeta,
    pub spec: RevisionSpec,
    pub status: RevisionStatus,
}

#[derive(Default)]
pub struct RevisionSpec {
    pub containerConcurrency: Option<i32>,
    pub timeoutSeconds: Option<i32>,
    pub template: RevisionTemplateSpec,
}

#[derive(Default)]
pub struct RevisionStatus {
    pub actualReplicas: Option<i32>,
    pub desiredReplicas: Option<i32>,
    pub traffic: Vec<TrafficTarget>,
}

impl Revision {
    pub fn new(_tenant_id: &str) -> Self {
        unimplemented!("cave-knative::revision::Revision::new")
    }

    pub fn scale_to_zero(&mut self) {
        unimplemented!("cave-knative::revision::Revision::scale_to_zero")
    }

    pub fn name(&self) -> String {
        unimplemented!("cave-knative::revision::Revision::name")
    }
}

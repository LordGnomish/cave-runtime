//! Knative Service (ksvc) — top-level CRD wrapping Configuration + Route.
//! upstream: knative/serving v1.18.x — pkg/apis/serving/v1/service_types.go

use crate::meta::{ObjectMeta, RevisionTemplateSpec, TrafficTarget};

#[derive(Default)]
pub struct Ksvc {
    pub metadata: ObjectMeta,
    pub spec: ServiceSpec,
    pub status: ServiceStatus,
}

#[derive(Default)]
pub struct ServiceSpec {
    pub template: RevisionTemplateSpec,
    pub traffic: Vec<TrafficTarget>,
}

#[derive(Default)]
pub struct ServiceStatus {
    pub traffic: Vec<TrafficTarget>,
}

impl Ksvc {
    pub fn new(_tenant_id: &str) -> Self {
        unimplemented!("cave-knative::ksvc::Ksvc::new")
    }

    pub fn scale_to_zero(&mut self) {
        unimplemented!("cave-knative::ksvc::Ksvc::scale_to_zero")
    }

    pub fn name(&self) -> String {
        unimplemented!("cave-knative::ksvc::Ksvc::name")
    }
}

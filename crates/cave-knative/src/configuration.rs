//! Knative Configuration — desired state spawning Revisions.
//! upstream: knative/serving v1.18.x — pkg/apis/serving/v1/configuration_types.go

use crate::meta::{ObjectMeta, RevisionTemplateSpec};

#[derive(Default)]
pub struct Configuration {
    pub metadata: ObjectMeta,
    pub spec: ConfigurationSpec,
    pub status: ConfigurationStatus,
}

#[derive(Default)]
pub struct ConfigurationSpec {
    pub template: RevisionTemplateSpec,
}

#[derive(Default)]
pub struct ConfigurationStatus {
    pub latestCreatedRevisionName: Option<String>,
    pub latestReadyRevisionName: Option<String>,
}

impl Configuration {
    pub fn new(_tenant_id: &str) -> Self {
        unimplemented!("cave-knative::configuration::Configuration::new")
    }

    pub fn scale_to_zero(&mut self) {
        unimplemented!("cave-knative::configuration::Configuration::scale_to_zero")
    }
}

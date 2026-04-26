//! Knative Eventing — Source/Sink/Broker/Trigger primitives.
//! upstream: knative/eventing v1.18.x — pkg/apis/sources/v1/

use crate::meta::ObjectMeta;

#[derive(Default)]
pub struct EventingSource {
    pub metadata: ObjectMeta,
    pub status: EventingSourceStatus,
}

#[derive(Default)]
pub struct EventingSourceStatus {
    pub sinkURI: Option<String>,
    pub ceAttributes: Vec<String>,
}

#[derive(Default)]
pub struct EventingSink {
    pub metadata: ObjectMeta,
}

impl EventingSource {
    pub fn new(_tenant_id: &str) -> Self {
        unimplemented!("cave-knative::eventing::EventingSource::new")
    }

    pub fn scale_to_zero(&mut self) {
        unimplemented!("cave-knative::eventing::EventingSource::scale_to_zero")
    }
}

impl EventingSink {
    pub fn new(_tenant_id: &str) -> Self {
        unimplemented!("cave-knative::eventing::EventingSink::new")
    }
}

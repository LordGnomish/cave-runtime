//! ScaledObject CRD — autoscale Deployment/StatefulSet/Custom workloads.
//! upstream: kedacore/keda v2.x — apis/keda/v1alpha1/scaledobject_types.go

use std::time::Duration;

#[derive(Default)]
pub struct ScaledObject {
    pub tenant_id: String,
    pub min_replica_count: Option<i32>,
    pub max_replica_count: Option<i32>,
    pub polling_interval: Option<Duration>,
    pub cooldown_period: Option<Duration>,
}

impl ScaledObject {
    pub fn new(_tenant_id: &str) -> Self {
        unimplemented!("cave-keda::scaledobject::ScaledObject::new")
    }

    pub fn scale_to_zero(&mut self) {
        unimplemented!("cave-keda::scaledobject::ScaledObject::scale_to_zero")
    }
}

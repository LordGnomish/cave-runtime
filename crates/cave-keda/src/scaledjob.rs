//! ScaledJob CRD — autoscale Job workloads with per-event Job creation.
//! upstream: kedacore/keda v2.x — apis/keda/v1alpha1/scaledjob_types.go

use std::time::Duration;

#[derive(Default)]
pub struct ScaledJob {
    pub tenant_id: String,
    pub max_replica_count: Option<i32>,
    pub polling_interval: Option<Duration>,
    pub successful_jobs_history_limit: Option<i32>,
    pub failed_jobs_history_limit: Option<i32>,
}

impl ScaledJob {
    pub fn new(_tenant_id: &str) -> Self {
        unimplemented!("cave-keda::scaledjob::ScaledJob::new")
    }
}

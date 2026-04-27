//! Cron scaler — schedule-based scaling.
//! upstream: kedacore/keda v2.x — pkg/scalers/cron_scaler.go

#[derive(Default)]
pub struct CronScaler {
    pub tenant_id: String,
    pub timezone: String,
    pub start_schedule: String,
    pub end_schedule: String,
    pub desired_replicas: Option<i32>,
}

impl CronScaler {
    pub fn new(_tenant_id: &str) -> Self {
        unimplemented!("cave-keda::cron_scaler::CronScaler::new")
    }
}

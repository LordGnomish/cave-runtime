//! cave-keda: KEDA event-driven autoscaler reimpl (scaffold — impl pending).
//!
//! upstream: kedacore/keda v2.x

pub mod scaler;
pub mod scaledobject;
pub mod scaledjob;
pub mod trigger_authentication;
pub mod http_scaler;
pub mod cron_scaler;
pub mod kafka_scaler;

pub use scaler::{Scaler, ScalingModifiers};
pub use scaledobject::ScaledObject;
pub use scaledjob::ScaledJob;
pub use trigger_authentication::TriggerAuthentication;
pub use http_scaler::HttpScaler;
pub use cron_scaler::CronScaler;
pub use kafka_scaler::KafkaScaler;

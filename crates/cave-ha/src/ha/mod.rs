pub mod failover;
pub mod health;

pub use failover::{FailoverConfig, FailoverEvent, FailoverManager, SplitBrainGuard};
pub use health::{HealthRegistry, NodeHealth, SystemProbe};

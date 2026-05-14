// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
pub mod failover;
pub mod health;

pub use failover::{FailoverConfig, FailoverEvent, FailoverManager, SplitBrainGuard};
pub use health::{HealthRegistry, NodeHealth, SystemProbe};

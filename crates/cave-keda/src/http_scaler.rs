// SPDX-License-Identifier: AGPL-3.0-or-later
//! HTTP scaler (KEDA HTTP add-on) — scales on inbound HTTP request rate.
//! upstream: kedacore/http-add-on v0.x

use crate::scaler::ScalerTrait;
use std::time::Duration;

#[derive(Default, Debug, Clone)]
pub struct HttpScaler {
    pub tenant_id: String,
    pub host: String,
    pub target_pending_requests: Option<i32>,
    pub current_pending_requests: i64,
}

impl HttpScaler {
    pub fn new(tenant_id: &str) -> Self {
        Self {
            tenant_id: tenant_id.to_string(),
            host: String::new(),
            target_pending_requests: Some(100),
            current_pending_requests: 0,
        }
    }

    pub fn observe(&mut self, pending: i64) {
        self.current_pending_requests = pending.max(0);
    }
}

impl ScalerTrait for HttpScaler {
    fn metric_value(&self) -> Option<f64> {
        Some(self.current_pending_requests as f64)
    }

    fn is_active(&self) -> bool {
        self.current_pending_requests > 0
    }

    fn polling_interval(&self) -> Duration {
        Duration::from_secs(15)
    }
}

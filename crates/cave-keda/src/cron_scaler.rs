// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Cron scaler — schedule-based scaling.
//! upstream: kedacore/keda v2.x — pkg/scalers/cron_scaler.go

use crate::scaler::ScalerTrait;
use std::time::Duration;

#[derive(Default, Debug, Clone)]
pub struct CronScaler {
    pub tenant_id: String,
    pub timezone: String,
    /// Cron expression for "scale up" — when this fires, the scaler is active.
    pub start_schedule: String,
    /// Cron expression for "scale down" — when this fires, the scaler is inactive.
    pub end_schedule: String,
    /// Replicas to apply during the active window.
    pub desired_replicas: Option<i32>,
    /// Whether the current time falls inside the active window
    /// (set by the controller via tick()).
    pub active: bool,
}

impl CronScaler {
    pub fn new(tenant_id: &str) -> Self {
        Self {
            tenant_id: tenant_id.to_string(),
            timezone: "UTC".to_string(),
            start_schedule: "0 9 * * *".to_string(),
            end_schedule: "0 17 * * *".to_string(),
            desired_replicas: Some(1),
            active: false,
        }
    }

    pub fn set_active(&mut self, active: bool) {
        self.active = active;
    }
}

impl ScalerTrait for CronScaler {
    fn metric_value(&self) -> Option<f64> {
        if self.active {
            self.desired_replicas.map(|r| r as f64)
        } else {
            Some(0.0)
        }
    }

    fn is_active(&self) -> bool {
        self.active
    }

    fn polling_interval(&self) -> Duration {
        Duration::from_secs(60)
    }
}

/// Best-effort cron-expression validator. Accepts standard 5-field cron
/// (minute hour day-of-month month day-of-week). Returns Err with the
/// position of the first invalid field.
pub fn validate_cron(expr: &str) -> Result<(), String> {
    let fields: Vec<&str> = expr.split_whitespace().collect();
    if fields.len() != 5 {
        return Err(format!("expected 5 cron fields, got {}", fields.len()));
    }
    let ranges = [(0, 59), (0, 23), (1, 31), (1, 12), (0, 6)];
    for (i, f) in fields.iter().enumerate() {
        if *f == "*" {
            continue;
        }
        if let Some(stripped) = f.strip_prefix("*/") {
            // step value
            stripped
                .parse::<u32>()
                .map_err(|_| format!("field {i}: invalid step value '{f}'"))?;
            continue;
        }
        for chunk in f.split(',') {
            if let Some((a, b)) = chunk.split_once('-') {
                let (_min, max) = ranges[i];
                let a: u32 = a.parse().map_err(|_| format!("field {i}: '{a}'"))?;
                let b: u32 = b.parse().map_err(|_| format!("field {i}: '{b}'"))?;
                if a > b || b > max {
                    return Err(format!("field {i}: range {a}-{b} out of bounds"));
                }
            } else {
                let v: u32 = chunk.parse().map_err(|_| format!("field {i}: '{chunk}'"))?;
                let (min, max) = ranges[i];
                if v < min || v > max {
                    return Err(format!("field {i}: value {v} out of bounds"));
                }
            }
        }
    }
    Ok(())
}

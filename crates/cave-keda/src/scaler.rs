//! Scaler trait + ScalingModifiers.
//! upstream: kedacore/keda v2.x — pkg/scalers/

use std::time::Duration;

#[derive(Default)]
pub struct Scaler {
    pub tenant_id: String,
    pub polling_interval: Option<Duration>,
    pub cooldown_period: Option<Duration>,
}

impl Scaler {
    pub fn new(_tenant_id: &str) -> Self {
        unimplemented!("cave-keda::scaler::Scaler::new")
    }

    pub fn scale_to_zero(&mut self) {
        unimplemented!("cave-keda::scaler::Scaler::scale_to_zero")
    }

    pub fn fallback(&self) -> Option<i32> {
        unimplemented!("cave-keda::scaler::Scaler::fallback")
    }
}

#[derive(Default)]
pub struct ScalingModifiers {
    pub formula: Option<String>,
    pub target: Option<i32>,
    pub activation_target: Option<i32>,
}

//! Cooldown tracking — prevents oscillation on scale-down.

use chrono::{DateTime, Utc};
use dashmap::DashMap;

pub struct CooldownTracker {
    last_scale: DashMap<String, DateTime<Utc>>,
}

impl CooldownTracker {
    pub fn new() -> Self {
        Self { last_scale: DashMap::new() }
    }

    pub fn record_scale(&self, ns: &str, name: &str) {
        let key = format!("{ns}/{name}");
        self.last_scale.insert(key, Utc::now());
    }

    pub fn is_in_cooldown(&self, ns: &str, name: &str, period_secs: u32) -> bool {
        let key = format!("{ns}/{name}");
        match self.last_scale.get(&key) {
            None => false,
            Some(last) => {
                let elapsed = Utc::now().signed_duration_since(*last);
                elapsed.num_seconds() < period_secs as i64
            }
        }
    }

    pub fn remaining_secs(&self, ns: &str, name: &str, period_secs: u32) -> u32 {
        let key = format!("{ns}/{name}");
        match self.last_scale.get(&key) {
            None => 0,
            Some(last) => {
                let elapsed = Utc::now().signed_duration_since(*last).num_seconds();
                let remaining = period_secs as i64 - elapsed;
                remaining.max(0) as u32
            }
        }
    }
}

impl Default for CooldownTracker {
    fn default() -> Self { Self::new() }
}

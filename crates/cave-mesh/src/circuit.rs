// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Circuit breaker (Closed → Open → HalfOpen → Closed).
//!
//! Configured per (host, optional subset).  Transitions:
//!   Closed   → Open      after `consecutive_errors` failures in a row
//!   Open     → HalfOpen  after `base_ejection_time` has elapsed (linear back-off)
//!   HalfOpen → Closed    on the next successful probe
//!   HalfOpen → Open      on the next failed probe
//!
//! Sweep-011 adoption: the per-ejection wait schedule
//! (`base * (1 + reopen_count)` capped at `max_ejection_time`) is now
//! produced by `cave_kernel::backoff::Backoff::Linear`. Behaviour is
//! byte-for-byte identical; the kernel primitive becomes the single
//! source of truth for the schedule shape so future tweaks (Fibonacci
//! / Exponential) are one-line swaps.

use cave_kernel::backoff::Backoff;
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};
use tracing::{info, warn};

// ─────────────────────────────────────────────────────────────
// Configuration
// ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct BreakerConfig {
    pub consecutive_errors: u32,
    pub max_connections: u32,
    pub max_pending_requests: u32,
    pub base_ejection_time: Duration,
    pub max_ejection_time: Duration,
    pub max_ejection_percent: u8,
}

impl Default for BreakerConfig {
    fn default() -> Self {
        Self {
            consecutive_errors: 5,
            max_connections: 1024,
            max_pending_requests: 1024,
            base_ejection_time: Duration::from_secs(30),
            max_ejection_time: Duration::from_secs(300),
            max_ejection_percent: 50,
        }
    }
}

// ─────────────────────────────────────────────────────────────
// State machine
// ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
enum BreakerState {
    Closed {
        consecutive_errors: u32,
    },
    Open {
        opened_at: Instant,
        ejection_duration: Duration,
    },
    HalfOpen,
}

#[derive(Debug, Clone)]
struct BreakerEntry {
    state: BreakerState,
    config: BreakerConfig,
    reopen_count: u32,
}

impl BreakerEntry {
    fn new(config: BreakerConfig) -> Self {
        Self {
            state: BreakerState::Closed {
                consecutive_errors: 0,
            },
            config,
            reopen_count: 0,
        }
    }
}

// ─────────────────────────────────────────────────────────────
// CircuitBreaker
// ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct CircuitBreaker {
    entries: Arc<RwLock<HashMap<String, BreakerEntry>>>,
}

impl Default for CircuitBreaker {
    fn default() -> Self {
        Self::new()
    }
}

impl CircuitBreaker {
    pub fn new() -> Self {
        Self {
            entries: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    fn make_key(host: &str, subset: Option<&str>) -> String {
        match subset {
            Some(s) if !s.is_empty() => format!("{host}/{s}"),
            _ => host.to_string(),
        }
    }

    pub fn configure(&self, host: &str, subset: Option<&str>, config: BreakerConfig) {
        let key = Self::make_key(host, subset);
        let mut map = self.entries.write().unwrap();
        map.entry(key)
            .and_modify(|e| e.config = config.clone())
            .or_insert_with(|| BreakerEntry::new(config));
    }

    /// Returns `true` if the circuit is open (requests should be shed).
    /// Automatically transitions Open → HalfOpen when ejection time expires.
    pub fn is_open(&self, host: &str, subset: Option<&str>) -> bool {
        let key = Self::make_key(host, subset);
        let mut map = self.entries.write().unwrap();
        let entry = map
            .entry(key.clone())
            .or_insert_with(|| BreakerEntry::new(BreakerConfig::default()));

        let (to_half_open, result) = match &entry.state {
            BreakerState::Closed { .. } => (false, false),
            BreakerState::HalfOpen => (false, false),
            BreakerState::Open {
                opened_at,
                ejection_duration,
            } => {
                if opened_at.elapsed() >= *ejection_duration {
                    (true, false)
                } else {
                    (false, true)
                }
            }
        };

        if to_half_open {
            info!(breaker = %key, "Circuit breaker → HalfOpen");
            entry.state = BreakerState::HalfOpen;
        }
        result
    }

    pub fn record_success(&self, host: &str, subset: Option<&str>) {
        let key = Self::make_key(host, subset);
        let mut map = self.entries.write().unwrap();
        let entry = map
            .entry(key.clone())
            .or_insert_with(|| BreakerEntry::new(BreakerConfig::default()));

        match entry.state {
            BreakerState::Closed {
                ref mut consecutive_errors,
            } => {
                *consecutive_errors = 0;
            }
            BreakerState::HalfOpen => {
                info!(breaker = %key, "Circuit breaker → Closed (probe succeeded)");
                entry.state = BreakerState::Closed {
                    consecutive_errors: 0,
                };
                entry.reopen_count = 0;
            }
            BreakerState::Open { .. } => {}
        }
    }

    pub fn record_failure(&self, host: &str, subset: Option<&str>) {
        let key = Self::make_key(host, subset);
        let mut map = self.entries.write().unwrap();
        let entry = map
            .entry(key.clone())
            .or_insert_with(|| BreakerEntry::new(BreakerConfig::default()));

        let threshold = entry.config.consecutive_errors;
        // Kernel-primitive backoff: `Backoff::Linear { base, cap }`
        // emits `base * (n+1)` capped at `cap`, byte-for-byte the
        // same as the prior `base.saturating_mul(1+reopen_count)`
        // expression.
        let backoff = Backoff::Linear {
            base: entry.config.base_ejection_time,
            cap: entry.config.max_ejection_time,
        };

        match &mut entry.state {
            BreakerState::Closed { consecutive_errors } => {
                *consecutive_errors += 1;
                if *consecutive_errors >= threshold {
                    let ejection = backoff.delay_for(entry.reopen_count);
                    warn!(
                        breaker = %key,
                        errors = *consecutive_errors,
                        ejection_secs = ejection.as_secs(),
                        "Circuit breaker → Open"
                    );
                    entry.state = BreakerState::Open {
                        opened_at: Instant::now(),
                        ejection_duration: ejection,
                    };
                    entry.reopen_count += 1;
                }
            }
            BreakerState::HalfOpen => {
                let ejection = backoff.delay_for(entry.reopen_count);
                warn!(breaker = %key, "Circuit breaker → Open (probe failed)");
                entry.state = BreakerState::Open {
                    opened_at: Instant::now(),
                    ejection_duration: ejection,
                };
                entry.reopen_count += 1;
            }
            BreakerState::Open { .. } => {}
        }
    }

    pub fn state_label(&self, host: &str, subset: Option<&str>) -> &'static str {
        let key = Self::make_key(host, subset);
        let map = self.entries.read().unwrap();
        match map.get(&key) {
            None => "closed",
            Some(e) => match &e.state {
                BreakerState::Closed { .. } => "closed",
                BreakerState::Open { .. } => "open",
                BreakerState::HalfOpen => "half_open",
            },
        }
    }

    pub fn snapshot(&self) -> Vec<BreakerSnapshot> {
        let map = self.entries.read().unwrap();
        map.iter()
            .map(|(k, e)| BreakerSnapshot {
                key: k.clone(),
                state: match &e.state {
                    BreakerState::Closed { consecutive_errors } => {
                        format!("closed (errors: {consecutive_errors})")
                    }
                    BreakerState::Open {
                        opened_at,
                        ejection_duration,
                    } => {
                        let remaining = ejection_duration
                            .checked_sub(opened_at.elapsed())
                            .unwrap_or_default();
                        format!("open ({}s remaining)", remaining.as_secs())
                    }
                    BreakerState::HalfOpen => "half_open".to_string(),
                },
                consecutive_errors: match &e.state {
                    BreakerState::Closed { consecutive_errors } => *consecutive_errors,
                    _ => 0,
                },
                max_connections: e.config.max_connections,
                max_pending_requests: e.config.max_pending_requests,
            })
            .collect()
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BreakerSnapshot {
    pub key: String,
    pub state: String,
    pub consecutive_errors: u32,
    pub max_connections: u32,
    pub max_pending_requests: u32,
}

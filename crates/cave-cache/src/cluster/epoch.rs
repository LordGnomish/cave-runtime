// SPDX-License-Identifier: AGPL-3.0-or-later
//! Configuration epoch counters.
//!
//! Mirrors `clusterGetCurrentEpoch` and the `currentEpoch` / `configEpoch`
//! monotonic counters from `src/cluster.c`. Both are 64-bit unsigned
//! counters that monotonically grow and break ties on conflicting
//! cluster updates: higher-epoch wins.

use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Default)]
pub struct EpochCounter {
    current: AtomicU64,
    config: AtomicU64,
}

impl EpochCounter {
    pub const fn new() -> Self {
        Self {
            current: AtomicU64::new(0),
            config: AtomicU64::new(0),
        }
    }

    pub fn from(current: u64, config: u64) -> Self {
        Self {
            current: AtomicU64::new(current),
            config: AtomicU64::new(config),
        }
    }

    pub fn current(&self) -> u64 {
        self.current.load(Ordering::Acquire)
    }

    pub fn config(&self) -> u64 {
        self.config.load(Ordering::Acquire)
    }

    /// Atomic-bump the current epoch, returning the new value.
    pub fn bump_current(&self) -> u64 {
        self.current.fetch_add(1, Ordering::AcqRel) + 1
    }

    pub fn bump_config(&self) -> u64 {
        self.config.fetch_add(1, Ordering::AcqRel) + 1
    }

    /// Adopt `observed` as the new floor of `current` iff it is
    /// strictly greater. Returns true when the value changed.
    pub fn observe_current(&self, observed: u64) -> bool {
        let mut cur = self.current.load(Ordering::Acquire);
        loop {
            if observed <= cur {
                return false;
            }
            match self.current.compare_exchange_weak(
                cur,
                observed,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return true,
                Err(c) => cur = c,
            }
        }
    }

    /// Adopt `observed` as the new floor of `config` iff strictly greater.
    pub fn observe_config(&self, observed: u64) -> bool {
        let mut cfg = self.config.load(Ordering::Acquire);
        loop {
            if observed <= cfg {
                return false;
            }
            match self.config.compare_exchange_weak(
                cfg,
                observed,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return true,
                Err(c) => cfg = c,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_at_zero() {
        let e = EpochCounter::new();
        assert_eq!(e.current(), 0);
        assert_eq!(e.config(), 0);
    }

    #[test]
    fn bump_current_increments() {
        let e = EpochCounter::new();
        assert_eq!(e.bump_current(), 1);
        assert_eq!(e.bump_current(), 2);
        assert_eq!(e.current(), 2);
    }

    #[test]
    fn bump_config_increments() {
        let e = EpochCounter::new();
        assert_eq!(e.bump_config(), 1);
        assert_eq!(e.bump_config(), 2);
        assert_eq!(e.config(), 2);
    }

    #[test]
    fn observe_current_adopts_higher_value() {
        let e = EpochCounter::new();
        assert!(e.observe_current(10));
        assert_eq!(e.current(), 10);
    }

    #[test]
    fn observe_current_ignores_lower_value() {
        let e = EpochCounter::from(20, 0);
        assert!(!e.observe_current(5));
        assert_eq!(e.current(), 20);
    }

    #[test]
    fn observe_current_ignores_equal_value() {
        let e = EpochCounter::from(7, 0);
        assert!(!e.observe_current(7));
    }

    #[test]
    fn observe_config_independent_of_current() {
        let e = EpochCounter::new();
        e.observe_config(50);
        assert_eq!(e.config(), 50);
        assert_eq!(e.current(), 0);
    }

    #[test]
    fn from_initializes_both_counters() {
        let e = EpochCounter::from(3, 4);
        assert_eq!(e.current(), 3);
        assert_eq!(e.config(), 4);
    }
}

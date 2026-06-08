// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Userspace mocks for `bpf_helpers.h`.
//!
//! Cilium programs call:
//!
//!   * `bpf_ktime_get_ns()` → monotonic clock; we use a virtual
//!     clock the test can advance deterministically.
//!   * `bpf_get_smp_processor_id()` → CPU id; we return 0 unless
//!     the test set it.
//!   * `bpf_redirect(ifindex, flags)` → datapath verdict; we model
//!     as a `Verdict::Redirect` enum.
//!   * `perf_event_output(ctx, map, flags, data, size)` → ring-
//!     buffer event; we collect into a `Vec<u8>` per map.
//!
//! Tests pin the clock via `MockClock::set` so timer-driven entry
//! expiry is deterministic.

use std::sync::{Arc, Mutex};

/// Virtual clock measured in nanoseconds since some arbitrary epoch.
/// Cilium programs treat the kernel's monotonic clock the same way —
/// only deltas matter, not absolute values.
#[derive(Debug, Clone)]
pub struct MockClock {
    now_ns: Arc<Mutex<u64>>,
}

impl Default for MockClock {
    fn default() -> Self {
        Self {
            now_ns: Arc::new(Mutex::new(0)),
        }
    }
}

impl MockClock {
    pub fn new(initial_ns: u64) -> Self {
        Self {
            now_ns: Arc::new(Mutex::new(initial_ns)),
        }
    }

    pub fn now_ns(&self) -> u64 {
        *self.now_ns.lock().expect("clock poisoned")
    }

    pub fn set(&self, ns: u64) {
        *self.now_ns.lock().expect("clock poisoned") = ns;
    }

    pub fn advance(&self, delta_ns: u64) {
        let mut g = self.now_ns.lock().expect("clock poisoned");
        *g = g.saturating_add(delta_ns);
    }
}

/// Aggregate handle a `Program::run` receives. Mirrors the
/// `__sk_buff *` / `struct xdp_md *` context in real BPF — but
/// reduced to "everything the simulator needs to make decisions".
#[derive(Debug, Clone)]
pub struct Helpers {
    pub clock: MockClock,
    pub cpu_id: u32,
    perf_events: Arc<Mutex<Vec<Vec<u8>>>>,
    /// `bpf_get_prandom_u32()` state. A test-injected queue is drained
    /// first (for deterministic port-allocation tests), then we fall
    /// back to a splitmix64 PRNG so untouched callers still get a
    /// varied-but-reproducible stream.
    prandom: Arc<Mutex<PrandomState>>,
}

#[derive(Debug)]
struct PrandomState {
    queue: std::collections::VecDeque<u32>,
    state: u64,
}

impl Default for Helpers {
    fn default() -> Self {
        Self {
            clock: MockClock::default(),
            cpu_id: 0,
            perf_events: Arc::new(Mutex::new(Vec::new())),
            prandom: Arc::new(Mutex::new(PrandomState {
                queue: std::collections::VecDeque::new(),
                // Non-zero seed; deterministic across runs.
                state: 0x9E37_79B9_7F4A_7C15,
            })),
        }
    }
}

impl Helpers {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_clock(mut self, clock: MockClock) -> Self {
        self.clock = clock;
        self
    }

    pub fn with_cpu(mut self, cpu_id: u32) -> Self {
        self.cpu_id = cpu_id;
        self
    }

    pub fn ktime_get_ns(&self) -> u64 {
        self.clock.now_ns()
    }

    pub fn get_smp_processor_id(&self) -> u32 {
        self.cpu_id
    }

    /// Append a perf event payload. Tests inspect via `perf_events`.
    pub fn perf_event_output(&self, data: &[u8]) {
        self.perf_events
            .lock()
            .expect("perf events poisoned")
            .push(data.to_vec());
    }

    pub fn perf_events(&self) -> Vec<Vec<u8>> {
        self.perf_events
            .lock()
            .expect("perf events poisoned")
            .clone()
    }

    pub fn perf_events_len(&self) -> usize {
        self.perf_events.lock().expect("perf events poisoned").len()
    }

    /// `bpf_get_prandom_u32()`. Returns the next test-injected value if
    /// one is queued, otherwise advances a splitmix64 PRNG. Cilium's
    /// datapath uses this for SNAT port selection and random backend
    /// selection — see `nat_sim` / `lb_sim`.
    pub fn get_prandom_u32(&self) -> u32 {
        let mut p = self.prandom.lock().expect("prandom poisoned");
        if let Some(v) = p.queue.pop_front() {
            return v;
        }
        // splitmix64
        p.state = p.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = p.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        ((z ^ (z >> 31)) >> 32) as u32
    }

    /// Inject one value to be returned by the next `get_prandom_u32`.
    /// Lets a test pin SNAT port selection / random backend choice.
    pub fn push_prandom(&self, value: u32) {
        self.prandom
            .lock()
            .expect("prandom poisoned")
            .queue
            .push_back(value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_clock_advances_and_reads() {
        let c = MockClock::new(1000);
        assert_eq!(c.now_ns(), 1000);
        c.advance(500);
        assert_eq!(c.now_ns(), 1500);
        c.set(42);
        assert_eq!(c.now_ns(), 42);
    }

    #[test]
    fn helpers_default_cpu_is_zero() {
        let h = Helpers::new();
        assert_eq!(h.get_smp_processor_id(), 0);
    }

    #[test]
    fn helpers_with_cpu_sets_id() {
        let h = Helpers::new().with_cpu(7);
        assert_eq!(h.get_smp_processor_id(), 7);
    }

    #[test]
    fn helpers_share_clock_when_cloned() {
        let h1 = Helpers::new().with_clock(MockClock::new(100));
        let h2 = h1.clone();
        h1.clock.advance(50);
        assert_eq!(h2.ktime_get_ns(), 150);
    }

    #[test]
    fn perf_event_output_accumulates_events() {
        let h = Helpers::new();
        h.perf_event_output(b"hello");
        h.perf_event_output(b"world");
        let events = h.perf_events();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0], b"hello");
        assert_eq!(events[1], b"world");
        assert_eq!(h.perf_events_len(), 2);
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Sharded alert notifier with per-Alertmanager retry budgets.
//!
//! upstream: prometheus/prometheus — pkg/notifier/notifier.go
//!
//! The upstream `Notifier` keeps one queue per Alertmanager peer, applies
//! a token-bucket retry budget per peer, and dispatches alert
//! notifications round-robin across peers that still have budget. We
//! port that shape: a `ShardedNotifier` owns a Vec<PeerQueue> indexed
//! by Alertmanager URL.

use std::collections::VecDeque;

/// One alert notification (an upstream `Alert` struct in shrunken form).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Notification {
    pub alertname: String,
    pub fingerprint: u64,
    pub state: String, // "firing" | "resolved"
}

impl Notification {
    pub fn new(alertname: &str, fp: u64, state: &str) -> Self {
        Self {
            alertname: alertname.to_string(),
            fingerprint: fp,
            state: state.to_string(),
        }
    }
}

/// One Alertmanager peer with its queue + token-bucket budget.
#[derive(Debug, Clone)]
pub struct PeerQueue {
    pub url: String,
    pub queue: VecDeque<Notification>,
    pub capacity: usize,
    /// Tokens currently available for sending (refilled by `tick`).
    pub tokens: u32,
    pub max_tokens: u32,
    pub refill_per_tick: u32,
    /// Per-peer counters surfaced to /metrics.
    pub sent_count: u64,
    pub dropped_count: u64,
    pub failed_count: u64,
}

impl PeerQueue {
    pub fn new(url: &str, capacity: usize, max_tokens: u32, refill_per_tick: u32) -> Self {
        Self {
            url: url.to_string(),
            queue: VecDeque::new(),
            capacity,
            tokens: max_tokens,
            max_tokens,
            refill_per_tick,
            sent_count: 0,
            dropped_count: 0,
            failed_count: 0,
        }
    }

    pub fn refill(&mut self) {
        self.tokens = (self.tokens + self.refill_per_tick).min(self.max_tokens);
    }

    pub fn enqueue(&mut self, n: Notification) -> bool {
        if self.queue.len() >= self.capacity {
            self.dropped_count += 1;
            return false;
        }
        self.queue.push_back(n);
        true
    }

    pub fn try_send_one<F>(&mut self, mut send_fn: F) -> Option<Notification>
    where
        F: FnMut(&str, &Notification) -> Result<(), ()>,
    {
        if self.tokens == 0 {
            return None;
        }
        let next = self.queue.pop_front()?;
        match send_fn(&self.url, &next) {
            Ok(()) => {
                self.tokens -= 1;
                self.sent_count += 1;
                Some(next)
            }
            Err(()) => {
                self.failed_count += 1;
                // re-queue at front so the next tick retries; do NOT consume token.
                self.queue.push_front(next.clone());
                None
            }
        }
    }
}

/// Round-robin dispatcher across multiple peers.
pub struct ShardedNotifier {
    pub peers: Vec<PeerQueue>,
    cursor: usize,
}

impl ShardedNotifier {
    pub fn new() -> Self {
        Self {
            peers: Vec::new(),
            cursor: 0,
        }
    }

    pub fn add_peer(&mut self, peer: PeerQueue) {
        self.peers.push(peer);
    }

    /// Enqueue an alert on each peer. Returns the number of peers that
    /// accepted (those whose queue still had room).
    pub fn broadcast(&mut self, n: &Notification) -> usize {
        let mut accepted = 0;
        for peer in self.peers.iter_mut() {
            if peer.enqueue(n.clone()) {
                accepted += 1;
            }
        }
        accepted
    }

    pub fn tick_refill(&mut self) {
        for p in self.peers.iter_mut() {
            p.refill();
        }
    }

    /// One drain pass — visits each peer round-robin and sends at most
    /// one notification. Returns the total number of notifications that
    /// actually shipped this pass.
    pub fn drain_round<F>(&mut self, mut send_fn: F) -> usize
    where
        F: FnMut(&str, &Notification) -> Result<(), ()>,
    {
        if self.peers.is_empty() {
            return 0;
        }
        let n = self.peers.len();
        let mut sent = 0;
        for offset in 0..n {
            let idx = (self.cursor + offset) % n;
            if self.peers[idx].try_send_one(&mut send_fn).is_some() {
                sent += 1;
            }
        }
        self.cursor = (self.cursor + 1) % n;
        sent
    }

    pub fn total_pending(&self) -> usize {
        self.peers.iter().map(|p| p.queue.len()).sum()
    }
}

impl Default for ShardedNotifier {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alert(name: &str) -> Notification {
        Notification::new(name, 1, "firing")
    }

    // ─── PeerQueue ─────────────────────────────────────────────────────

    #[test]
    fn peer_enqueue_until_capacity_then_drops() {
        let mut p = PeerQueue::new("am-1", 2, 8, 2);
        assert!(p.enqueue(alert("a")));
        assert!(p.enqueue(alert("b")));
        assert!(!p.enqueue(alert("c")));
        assert_eq!(p.dropped_count, 1);
    }

    #[test]
    fn peer_refill_caps_at_max_tokens() {
        let mut p = PeerQueue::new("am-1", 4, 5, 100);
        p.tokens = 0;
        p.refill();
        assert_eq!(p.tokens, 5);
    }

    #[test]
    fn peer_send_success_consumes_token() {
        let mut p = PeerQueue::new("am-1", 4, 2, 2);
        p.enqueue(alert("a"));
        let _ = p.try_send_one(|_, _| Ok(()));
        assert_eq!(p.tokens, 1);
        assert_eq!(p.sent_count, 1);
    }

    #[test]
    fn peer_send_failure_keeps_alert_and_counts_failed() {
        let mut p = PeerQueue::new("am-1", 4, 2, 2);
        p.enqueue(alert("a"));
        let _ = p.try_send_one(|_, _| Err(()));
        assert_eq!(p.queue.len(), 1, "failed alerts must be retained");
        assert_eq!(p.failed_count, 1);
        assert_eq!(p.tokens, 2, "no token consumed on failure");
    }

    #[test]
    fn peer_send_blocked_when_no_tokens() {
        let mut p = PeerQueue::new("am-1", 4, 0, 0);
        p.enqueue(alert("a"));
        assert!(p.try_send_one(|_, _| Ok(())).is_none());
        assert_eq!(p.queue.len(), 1);
    }

    // ─── ShardedNotifier ───────────────────────────────────────────────

    #[test]
    fn broadcast_enqueues_on_every_peer() {
        let mut n = ShardedNotifier::new();
        n.add_peer(PeerQueue::new("am-1", 4, 2, 2));
        n.add_peer(PeerQueue::new("am-2", 4, 2, 2));
        let accepted = n.broadcast(&alert("x"));
        assert_eq!(accepted, 2);
        assert_eq!(n.total_pending(), 2);
    }

    #[test]
    fn drain_round_visits_each_peer_once() {
        let mut n = ShardedNotifier::new();
        n.add_peer(PeerQueue::new("am-1", 4, 2, 2));
        n.add_peer(PeerQueue::new("am-2", 4, 2, 2));
        n.broadcast(&alert("a"));
        n.broadcast(&alert("b"));
        let sent = n.drain_round(|_, _| Ok(()));
        assert_eq!(sent, 2);
        assert_eq!(n.total_pending(), 2); // each peer still has 1 in queue
    }

    #[test]
    fn drain_round_cursor_rotates() {
        let mut n = ShardedNotifier::new();
        n.add_peer(PeerQueue::new("am-1", 4, 2, 2));
        n.add_peer(PeerQueue::new("am-2", 4, 2, 2));
        let c0 = 0;
        n.drain_round(|_, _| Ok(()));
        let c1 = 1;
        n.drain_round(|_, _| Ok(()));
        let c2 = 0;
        assert_eq!(n.cursor, c2);
        assert!(c0 < n.peers.len() && c1 < n.peers.len());
    }

    #[test]
    fn broadcast_skips_full_peer_but_counts_dropped() {
        let mut n = ShardedNotifier::new();
        let mut p1 = PeerQueue::new("am-1", 1, 2, 2);
        p1.enqueue(alert("filler"));
        n.add_peer(p1);
        n.add_peer(PeerQueue::new("am-2", 4, 2, 2));
        let accepted = n.broadcast(&alert("x"));
        assert_eq!(accepted, 1);
        assert_eq!(n.peers[0].dropped_count, 1);
        assert_eq!(n.peers[1].queue.len(), 1);
    }
}

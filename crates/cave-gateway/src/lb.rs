//! Load balancing algorithms: round-robin, least-connections,
//! consistent-hash (ketama), and weighted random.

use crate::models::{LbAlgorithm, Target};
use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// A resolved upstream endpoint.
#[derive(Debug, Clone)]
pub struct Endpoint {
    pub host: String,
    pub port: u16,
    pub weight: u32,
    pub target_id: uuid::Uuid,
}

impl From<&Target> for Endpoint {
    fn from(t: &Target) -> Self {
        let (host, port) = t.host_port();
        Endpoint {
            host: host.to_string(),
            port,
            weight: t.weight,
            target_id: t.id,
        }
    }
}

// ── Round-robin ───────────────────────────────────────────────────────────────

pub struct RoundRobin {
    cursor: AtomicUsize,
}

impl RoundRobin {
    pub fn new() -> Self {
        Self { cursor: AtomicUsize::new(0) }
    }

    pub fn pick<'a>(&self, endpoints: &'a [Endpoint]) -> Option<&'a Endpoint> {
        if endpoints.is_empty() {
            return None;
        }
        // Weighted: expand each endpoint weight times, then round-robin
        let total_weight: u32 = endpoints.iter().map(|e| e.weight.max(1)).sum();
        let idx = self.cursor.fetch_add(1, Ordering::Relaxed) % total_weight as usize;
        let mut acc = 0u32;
        for ep in endpoints {
            acc += ep.weight.max(1);
            if (idx as u32) < acc {
                return Some(ep);
            }
        }
        endpoints.first()
    }
}

// ── Least-connections ─────────────────────────────────────────────────────────

pub struct LeastConnections {
    pub counts: Arc<dashmap::DashMap<uuid::Uuid, usize>>,
}

impl LeastConnections {
    pub fn new() -> Self {
        Self { counts: Arc::new(dashmap::DashMap::new()) }
    }

    pub fn pick<'a>(&self, endpoints: &'a [Endpoint]) -> Option<&'a Endpoint> {
        endpoints.iter().min_by_key(|ep| {
            let c = self.counts.get(&ep.target_id).map(|v| *v).unwrap_or(0);
            // Weighted: divide connections by weight to normalize
            if ep.weight > 0 { c / ep.weight as usize } else { usize::MAX }
        })
    }

    pub fn inc(&self, id: uuid::Uuid) {
        *self.counts.entry(id).or_insert(0) += 1;
    }

    pub fn dec(&self, id: uuid::Uuid) {
        if let Some(mut v) = self.counts.get_mut(&id) {
            *v = v.saturating_sub(1);
        }
    }
}

// ── Consistent hashing (ketama) ───────────────────────────────────────────────

pub struct ConsistentHash {
    ring: Mutex<BTreeMap<u64, uuid::Uuid>>,
    replicas: usize,
}

impl ConsistentHash {
    pub fn new(replicas: usize) -> Self {
        Self { ring: Mutex::new(BTreeMap::new()), replicas }
    }

    pub fn rebuild(&self, endpoints: &[Endpoint]) {
        let mut ring = self.ring.lock().unwrap();
        ring.clear();
        for ep in endpoints {
            for i in 0..self.replicas {
                let key = format!("{}:{}:{}", ep.host, ep.port, i);
                let h = fnv_hash(key.as_bytes());
                ring.insert(h, ep.target_id);
            }
        }
    }

    pub fn pick<'a>(&self, endpoints: &'a [Endpoint], hash_key: u64) -> Option<&'a Endpoint> {
        let ring = self.ring.lock().unwrap();
        if ring.is_empty() {
            return endpoints.first();
        }
        // Find first node >= hash_key, or wrap around
        let id = ring
            .range(hash_key..)
            .next()
            .or_else(|| ring.iter().next())
            .map(|(_, id)| *id)?;
        endpoints.iter().find(|ep| ep.target_id == id)
    }
}

fn fnv_hash(data: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in data {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

pub fn hash_str(s: &str) -> u64 {
    fnv_hash(s.as_bytes())
}

// ── Balancer enum ─────────────────────────────────────────────────────────────

pub enum Balancer {
    RoundRobin(RoundRobin),
    LeastConnections(LeastConnections),
    ConsistentHash(ConsistentHash),
}

impl Balancer {
    pub fn new(algorithm: &LbAlgorithm) -> Self {
        match algorithm {
            LbAlgorithm::RoundRobin | LbAlgorithm::LatencyAware => {
                Balancer::RoundRobin(RoundRobin::new())
            }
            LbAlgorithm::LeastConnections => {
                Balancer::LeastConnections(LeastConnections::new())
            }
            LbAlgorithm::ConsistentHashing => {
                Balancer::ConsistentHash(ConsistentHash::new(150))
            }
        }
    }

    pub fn pick<'a>(&self, endpoints: &'a [Endpoint], hash_key: Option<u64>) -> Option<&'a Endpoint> {
        match self {
            Balancer::RoundRobin(rr) => rr.pick(endpoints),
            Balancer::LeastConnections(lc) => lc.pick(endpoints),
            Balancer::ConsistentHash(ch) => ch.pick(endpoints, hash_key.unwrap_or(0)),
        }
    }

    pub fn on_request_start(&self, target_id: uuid::Uuid) {
        if let Balancer::LeastConnections(lc) = self {
            lc.inc(target_id);
        }
    }

    pub fn on_request_end(&self, target_id: uuid::Uuid) {
        if let Balancer::LeastConnections(lc) = self {
            lc.dec(target_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn ep(host: &str, port: u16, weight: u32) -> Endpoint {
        Endpoint { host: host.into(), port, weight, target_id: Uuid::new_v4() }
    }

    #[test]
    fn round_robin_distributes() {
        let rr = RoundRobin::new();
        let eps = vec![ep("a", 80, 1), ep("b", 80, 1), ep("c", 80, 1)];
        let picks: Vec<_> = (0..9).map(|_| rr.pick(&eps).unwrap().host.clone()).collect();
        assert!(picks.contains(&"a".to_string()));
        assert!(picks.contains(&"b".to_string()));
        assert!(picks.contains(&"c".to_string()));
    }

    #[test]
    fn consistent_hash_same_key() {
        let ch = ConsistentHash::new(150);
        let eps = vec![ep("a", 80, 1), ep("b", 80, 1), ep("c", 80, 1)];
        ch.rebuild(&eps);
        let r1 = ch.pick(&eps, 12345).map(|e| e.host.clone());
        let r2 = ch.pick(&eps, 12345).map(|e| e.host.clone());
        assert_eq!(r1, r2);
    }

    #[test]
    fn weighted_round_robin() {
        let rr = RoundRobin::new();
        let eps = vec![ep("heavy", 80, 3), ep("light", 80, 1)];
        let mut heavy = 0;
        let mut light = 0;
        for _ in 0..40 {
            match rr.pick(&eps).unwrap().host.as_str() {
                "heavy" => heavy += 1,
                _ => light += 1,
            }
        }
        assert!(heavy > light * 2);
    }

    #[test]
    fn least_connections_picks_lowest() {
        let lc = LeastConnections::new();
        let id_a = Uuid::new_v4();
        let id_b = Uuid::new_v4();
        let eps = vec![
            Endpoint { host: "a".into(), port: 80, weight: 1, target_id: id_a },
            Endpoint { host: "b".into(), port: 80, weight: 1, target_id: id_b },
        ];
        lc.inc(id_a);
        lc.inc(id_a);
        let picked = lc.pick(&eps).unwrap();
        assert_eq!(picked.host, "b");
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Eventing transports — Kafka / RabbitMQ / Pulsar / NATS / GitHub Sources.
//!
//! upstream:
//!   - knative-extensions/eventing-kafka-broker
//!   - knative-extensions/eventing-rabbitmq
//!   - knative-extensions/eventing-natss
//!   - knative-sandbox/eventing-pulsar
//!   - knative-extensions/eventing-github
//!
//! A Knative `Channel` is an abstract subscribable+addressable resource.
//! Each transport plugin teaches the broker how to fan events out using
//! its native primitives (topic, queue, subject, etc.). Upstream lives
//! out-of-repo in `knative-extensions/*`; we collapse the surface into a
//! single `Transport` trait + four concrete back-ends and one webhook
//! source adapter.
//!
//! Networking is mocked through an in-memory routing table so the unit
//! tests are deterministic; the real transports live in cave-streams
//! (Kafka/Pulsar) and cave-runtime (NATS/RabbitMQ data-plane).

use crate::sources_ping::CloudEvent;
use std::collections::HashMap;

/// Result of publishing one CloudEvent through a transport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryReceipt {
    /// Logical destination (topic / queue / subject / partition key).
    pub destination: String,
    /// 1-based delivery attempt counter for the current send.
    pub attempt: u32,
    /// True on the first attempt that succeeded.
    pub delivered: bool,
}

/// Common transport interface used by the broker.
pub trait Transport: Send + Sync {
    fn name(&self) -> &'static str;
    fn publish(&mut self, destination: &str, event: &CloudEvent) -> DeliveryReceipt;
    /// Logical name of an underlying address (e.g. "topic/foo", "amqp://q/bar").
    fn address(&self, destination: &str) -> String;
}

// ─────────────────────────────────────────────────────────────────────────────
// Kafka
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Default, Debug, Clone)]
pub struct KafkaTransport {
    pub bootstrap_servers: String,
    pub partitions_per_topic: u32,
    pub log: Vec<(String, CloudEvent)>,
}

impl KafkaTransport {
    pub fn new(bootstrap: &str) -> Self {
        Self {
            bootstrap_servers: bootstrap.to_string(),
            partitions_per_topic: 1,
            log: Vec::new(),
        }
    }

    /// Pick a partition for a given event using extension `partitionkey`,
    /// falling back to the event id. Hash is a simple FNV-1a so we don't
    /// pull in std::hash::Hasher type plumbing.
    pub fn pick_partition(&self, event: &CloudEvent) -> u32 {
        if self.partitions_per_topic <= 1 {
            return 0;
        }
        let key = event
            .extensions
            .get("partitionkey")
            .map(String::as_str)
            .unwrap_or(event.id.as_str());
        let mut h: u32 = 0x811c9dc5;
        for b in key.bytes() {
            h ^= b as u32;
            h = h.wrapping_mul(0x01000193);
        }
        h % self.partitions_per_topic
    }
}

impl Transport for KafkaTransport {
    fn name(&self) -> &'static str {
        "kafka"
    }
    fn publish(&mut self, destination: &str, event: &CloudEvent) -> DeliveryReceipt {
        let part = self.pick_partition(event);
        let topic = format!("knative-channel-{}", destination);
        self.log.push((topic.clone(), event.clone()));
        DeliveryReceipt {
            destination: format!("{}#p{}", topic, part),
            attempt: 1,
            delivered: true,
        }
    }
    fn address(&self, destination: &str) -> String {
        format!("kafka://{}/{}", self.bootstrap_servers, destination)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// RabbitMQ
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Default, Debug, Clone)]
pub struct RabbitMqTransport {
    pub broker_uri: String,
    /// queues binding -> messages (in arrival order).
    pub queues: HashMap<String, Vec<CloudEvent>>,
    pub attempts: HashMap<String, u32>,
}

impl RabbitMqTransport {
    pub fn new(uri: &str) -> Self {
        Self {
            broker_uri: uri.to_string(),
            queues: HashMap::new(),
            attempts: HashMap::new(),
        }
    }
}

impl Transport for RabbitMqTransport {
    fn name(&self) -> &'static str {
        "rabbitmq"
    }
    fn publish(&mut self, destination: &str, event: &CloudEvent) -> DeliveryReceipt {
        let q = format!("knative-{}", destination);
        let entry = self.attempts.entry(q.clone()).or_insert(0);
        *entry += 1;
        let attempt = *entry;
        self.queues
            .entry(q.clone())
            .or_default()
            .push(event.clone());
        DeliveryReceipt {
            destination: q,
            attempt,
            delivered: true,
        }
    }
    fn address(&self, destination: &str) -> String {
        format!(
            "{}/knative-{}",
            self.broker_uri.trim_end_matches('/'),
            destination
        )
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Pulsar
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Default, Debug, Clone)]
pub struct PulsarTransport {
    pub service_url: String,
    pub tenant: String,
    pub namespace: String,
    pub log: Vec<(String, CloudEvent)>,
}

impl PulsarTransport {
    pub fn new(service_url: &str, tenant: &str, namespace: &str) -> Self {
        Self {
            service_url: service_url.to_string(),
            tenant: tenant.to_string(),
            namespace: namespace.to_string(),
            log: Vec::new(),
        }
    }
    pub fn topic_of(&self, destination: &str) -> String {
        format!(
            "persistent://{}/{}/knative-{}",
            self.tenant, self.namespace, destination
        )
    }
}

impl Transport for PulsarTransport {
    fn name(&self) -> &'static str {
        "pulsar"
    }
    fn publish(&mut self, destination: &str, event: &CloudEvent) -> DeliveryReceipt {
        let topic = self.topic_of(destination);
        self.log.push((topic.clone(), event.clone()));
        DeliveryReceipt {
            destination: topic,
            attempt: 1,
            delivered: true,
        }
    }
    fn address(&self, destination: &str) -> String {
        self.topic_of(destination)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// NATS / NATSS
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Default, Debug, Clone)]
pub struct NatsTransport {
    pub urls: Vec<String>,
    pub stream_prefix: String,
    pub subjects: HashMap<String, Vec<CloudEvent>>,
}

impl NatsTransport {
    pub fn new(url: &str) -> Self {
        Self {
            urls: vec![url.to_string()],
            stream_prefix: "KNATIVE".to_string(),
            subjects: HashMap::new(),
        }
    }
    pub fn subject_of(&self, destination: &str) -> String {
        format!("{}.{}", self.stream_prefix, destination)
    }
}

impl Transport for NatsTransport {
    fn name(&self) -> &'static str {
        "nats"
    }
    fn publish(&mut self, destination: &str, event: &CloudEvent) -> DeliveryReceipt {
        let subj = self.subject_of(destination);
        self.subjects
            .entry(subj.clone())
            .or_default()
            .push(event.clone());
        DeliveryReceipt {
            destination: subj,
            attempt: 1,
            delivered: true,
        }
    }
    fn address(&self, destination: &str) -> String {
        let base = self
            .urls
            .first()
            .cloned()
            .unwrap_or_else(|| "nats://localhost:4222".to_string());
        format!("{}/{}", base, self.subject_of(destination))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// GitHub Source — emits CloudEvents on incoming webhook payload
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Default, Debug, Clone)]
pub struct GitHubSource {
    pub owner_and_repo: String,
    pub event_types: Vec<String>,
    pub secret_token: Option<String>,
    pub sink: Option<String>,
}

impl GitHubSource {
    pub fn new(owner_and_repo: &str) -> Self {
        Self {
            owner_and_repo: owner_and_repo.to_string(),
            ..Default::default()
        }
    }

    /// Validate a webhook payload against the configured event types and
    /// (optionally) the shared HMAC token. Returns a CloudEvent envelope
    /// when the payload is accepted.
    pub fn accept(
        &self,
        github_event_header: &str,
        body: &str,
        signature: Option<&str>,
        id: &str,
    ) -> Option<CloudEvent> {
        if !self.event_types.is_empty()
            && !self.event_types.iter().any(|t| t == github_event_header)
        {
            return None;
        }
        if let Some(ref tok) = self.secret_token {
            let want = hmac_sha256_hex(tok.as_bytes(), body.as_bytes());
            if signature.map(|s| s.trim_start_matches("sha256=").eq_ignore_ascii_case(&want))
                != Some(true)
            {
                return None;
            }
        }
        let mut extensions: HashMap<String, String> = HashMap::new();
        extensions.insert("githuborg".to_string(), self.owner_and_repo.clone());
        Some(CloudEvent {
            id: id.to_string(),
            source: format!(
                "/apis/sources.knative.dev/v1/githubsources/{}",
                self.owner_and_repo
            ),
            spec_version: "1.0".to_string(),
            event_type: format!("dev.knative.source.github.{}", github_event_header),
            content_type: "application/json".to_string(),
            data: Some(body.to_string()),
            extensions,
        })
    }
}

/// HMAC-SHA256 — small dependency-free implementation. Returns the
/// lowercase-hex digest that matches GitHub's `X-Hub-Signature-256`.
pub fn hmac_sha256_hex(key: &[u8], msg: &[u8]) -> String {
    let mut k = [0u8; 64];
    if key.len() > 64 {
        let h = sha256(key);
        k[..32].copy_from_slice(&h);
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut ipad = [0x36u8; 64];
    let mut opad = [0x5cu8; 64];
    for i in 0..64 {
        ipad[i] ^= k[i];
        opad[i] ^= k[i];
    }
    let mut inner = Vec::with_capacity(64 + msg.len());
    inner.extend_from_slice(&ipad);
    inner.extend_from_slice(msg);
    let inner_hash = sha256(&inner);
    let mut outer = Vec::with_capacity(64 + 32);
    outer.extend_from_slice(&opad);
    outer.extend_from_slice(&inner_hash);
    let out = sha256(&outer);
    let mut hex = String::with_capacity(64);
    for b in out {
        hex.push_str(&format!("{:02x}", b));
    }
    hex
}

/// SHA-256 — minimal pure-Rust implementation (RFC 6234).
pub fn sha256(input: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];

    let bit_len = (input.len() as u64).wrapping_mul(8);
    let mut data: Vec<u8> = input.to_vec();
    data.push(0x80);
    while data.len() % 64 != 56 {
        data.push(0);
    }
    data.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in data.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let mut a = h[0];
        let mut b = h[1];
        let mut c = h[2];
        let mut d = h[3];
        let mut e = h[4];
        let mut f = h[5];
        let mut g = h[6];
        let mut hh = h[7];
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }
    let mut out = [0u8; 32];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources_ping::PingSource;

    fn mk_event(id: &str) -> CloudEvent {
        PingSource::new("t", "*/1 * * * *").emit(id)
    }

    // ─── Kafka ───────────────────────────────────────────────────────────

    #[test]
    fn kafka_publishes_to_channel_topic() {
        let mut k = KafkaTransport::new("localhost:9092");
        let r = k.publish("orders", &mk_event("ev-1"));
        assert!(r.delivered);
        assert!(r.destination.starts_with("knative-channel-orders"));
        assert_eq!(k.log.len(), 1);
    }

    #[test]
    fn kafka_partition_uses_partitionkey_extension() {
        let mut k = KafkaTransport::new("b");
        k.partitions_per_topic = 4;
        let mut ev1 = mk_event("a");
        let mut ev2 = mk_event("b");
        ev1.extensions
            .insert("partitionkey".to_string(), "tenant-x".to_string());
        ev2.extensions
            .insert("partitionkey".to_string(), "tenant-x".to_string());
        assert_eq!(k.pick_partition(&ev1), k.pick_partition(&ev2));
    }

    #[test]
    fn kafka_partition_falls_back_to_event_id() {
        let mut k = KafkaTransport::new("b");
        k.partitions_per_topic = 8;
        let ev = mk_event("a-stable-key");
        let p1 = k.pick_partition(&ev);
        let p2 = k.pick_partition(&ev);
        assert_eq!(p1, p2);
        assert!(p1 < 8);
    }

    // ─── RabbitMQ ────────────────────────────────────────────────────────

    #[test]
    fn rabbitmq_queue_per_destination() {
        let mut r = RabbitMqTransport::new("amqp://localhost:5672/");
        let receipt = r.publish("orders", &mk_event("e"));
        assert!(receipt.destination.starts_with("knative-orders"));
        assert_eq!(r.queues["knative-orders"].len(), 1);
    }

    #[test]
    fn rabbitmq_attempt_counter_bumps_per_destination() {
        let mut r = RabbitMqTransport::new("amqp://x/");
        let _ = r.publish("d", &mk_event("e1"));
        let _ = r.publish("d", &mk_event("e2"));
        let receipt = r.publish("d", &mk_event("e3"));
        assert_eq!(receipt.attempt, 3);
    }

    // ─── Pulsar ──────────────────────────────────────────────────────────

    #[test]
    fn pulsar_uses_persistent_address_with_tenant_ns() {
        let mut p = PulsarTransport::new("pulsar://localhost:6650", "tenant-x", "ns-y");
        let r = p.publish("orders", &mk_event("e"));
        assert!(
            r.destination
                .starts_with("persistent://tenant-x/ns-y/knative-orders")
        );
    }

    #[test]
    fn pulsar_topic_format_stable() {
        let p = PulsarTransport::new("pulsar://h:6650", "t", "n");
        assert_eq!(p.topic_of("foo"), "persistent://t/n/knative-foo");
    }

    // ─── NATS ────────────────────────────────────────────────────────────

    #[test]
    fn nats_subject_prefixed_with_knative() {
        let mut n = NatsTransport::new("nats://localhost:4222");
        let r = n.publish("clicks", &mk_event("e"));
        assert_eq!(r.destination, "KNATIVE.clicks");
        assert_eq!(n.subjects["KNATIVE.clicks"].len(), 1);
    }

    // ─── GitHub ──────────────────────────────────────────────────────────

    #[test]
    fn github_event_type_filter_accept_only_listed() {
        let mut g = GitHubSource::new("acme/cave");
        g.event_types = vec!["push".to_string()];
        assert!(g.accept("push", "{}", None, "id").is_some());
        assert!(g.accept("issues", "{}", None, "id").is_none());
    }

    #[test]
    fn github_hmac_signature_required_when_token_set() {
        let mut g = GitHubSource::new("acme/cave");
        g.secret_token = Some("topsecret".to_string());
        let body = "{\"action\":\"opened\"}";
        let sig = hmac_sha256_hex(b"topsecret", body.as_bytes());
        let header = format!("sha256={}", sig);
        assert!(g.accept("issues", body, Some(&header), "id-1").is_some());
        assert!(
            g.accept("issues", body, Some("sha256=deadbeef"), "id-1")
                .is_none()
        );
        assert!(g.accept("issues", body, None, "id-1").is_none());
    }

    #[test]
    fn github_event_carries_type_prefix() {
        let g = GitHubSource::new("acme/cave");
        let ev = g.accept("pull_request", "{}", None, "id").unwrap();
        assert_eq!(ev.event_type, "dev.knative.source.github.pull_request");
        assert_eq!(
            ev.extensions.get("githuborg").map(|s| s.as_str()),
            Some("acme/cave")
        );
    }

    // ─── SHA-256 cross-check ────────────────────────────────────────────

    #[test]
    fn sha256_empty_string_matches_rfc6234() {
        let d = sha256(b"");
        let hex: String = d.iter().map(|b| format!("{:02x}", b)).collect();
        assert_eq!(
            hex,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn sha256_abc_matches_rfc6234() {
        let d = sha256(b"abc");
        let hex: String = d.iter().map(|b| format!("{:02x}", b)).collect();
        assert_eq!(
            hex,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn hmac_sha256_matches_rfc4231_test1() {
        // key = 0x0b * 20, msg = "Hi There" → 0xb0344c61d8db38535ca8afceaf0bf12b...
        let key = [0x0bu8; 20];
        let h = hmac_sha256_hex(&key, b"Hi There");
        assert_eq!(
            h,
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
    }
}

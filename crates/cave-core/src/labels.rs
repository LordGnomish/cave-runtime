//! Canonical label set for the Cave observability stack.
//!
//! Both cave-logs (Loki-style streams) and cave-metrics (Prometheus-style
//! time series) need the same primitive: an ordered key-value map with a
//! stable 64-bit fingerprint.
//!
//! # Upstream reference
//! Fingerprint algorithm: FNV-1a 64-bit (same as Prometheus `labels.go`).
//! Selector format: `{k="v",...}` (same as Loki/PromQL label selector syntax).

use std::collections::BTreeMap;
use serde::{Deserialize, Serialize};

// ── Types ─────────────────────────────────────────────────────────────────────

/// An ordered, stable set of `key=value` label pairs.
///
/// Uses `BTreeMap` so iteration order is deterministic — required for a stable
/// `fingerprint()` without explicit sorting.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default)]
pub struct Labels(pub BTreeMap<String, String>);

impl Labels {
    pub fn new() -> Self {
        Self(BTreeMap::new())
    }

    pub fn from_pairs(
        pairs: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
    ) -> Self {
        Self(pairs.into_iter().map(|(k, v)| (k.into(), v.into())).collect())
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.get(name).map(|s| s.as_str())
    }

    pub fn insert(&mut self, name: impl Into<String>, value: impl Into<String>) {
        self.0.insert(name.into(), value.into());
    }

    pub fn remove(&mut self, name: &str) -> Option<String> {
        self.0.remove(name)
    }

    pub fn contains_key(&self, name: &str) -> bool {
        self.0.contains_key(name)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    /// Prometheus `__name__` label (metric name).
    pub fn metric_name(&self) -> Option<&str> {
        self.get("__name__")
    }

    /// Stable 64-bit FNV-1a fingerprint — identical algorithm to Prometheus.
    ///
    /// BTreeMap guarantees sorted iteration so no explicit sort is needed.
    pub fn fingerprint(&self) -> u64 {
        const FNV_OFFSET: u64 = 14_695_981_039_346_656_037;
        const FNV_PRIME: u64 = 1_099_511_628_211;
        let mut hash = FNV_OFFSET;
        for (k, v) in &self.0 {
            for byte in k
                .bytes()
                .chain(std::iter::once(b'='))
                .chain(v.bytes())
                .chain(std::iter::once(b','))
            {
                hash ^= byte as u64;
                hash = hash.wrapping_mul(FNV_PRIME);
            }
        }
        hash
    }

    /// Canonical selector string: `{k="v",...}` (keys sorted, no __name__).
    pub fn to_selector(&self) -> String {
        let inner: Vec<String> = self
            .0
            .iter()
            .filter(|(k, _)| k.as_str() != "__name__")
            .map(|(k, v)| format!("{}=\"{}\"", k, v))
            .collect();
        format!("{{{}}}", inner.join(","))
    }

    /// Return a copy without `__name__` (for grouping in aggregations).
    pub fn without_name(&self) -> Self {
        let mut m = self.0.clone();
        m.remove("__name__");
        Self(m)
    }

    /// Return a copy retaining only the specified keys.
    pub fn with_only(&self, keys: &[&str]) -> Self {
        Self(
            self.0
                .iter()
                .filter(|(k, _)| keys.contains(&k.as_str()))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        )
    }

    /// Return a copy excluding the specified keys.
    pub fn without(&self, keys: &[&str]) -> Self {
        Self(
            self.0
                .iter()
                .filter(|(k, _)| !keys.contains(&k.as_str()))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        )
    }
}

impl std::fmt::Display for Labels {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_selector())
    }
}

impl From<BTreeMap<String, String>> for Labels {
    fn from(m: BTreeMap<String, String>) -> Self {
        Self(m)
    }
}

impl From<std::collections::HashMap<String, String>> for Labels {
    fn from(m: std::collections::HashMap<String, String>) -> Self {
        Self(m.into_iter().collect())
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── upstream: TestLabelsFingerprint (Prometheus labels_test.go) ───────────

    #[test]
    fn test_fingerprint_stable() {
        let l1 = Labels::from_pairs([("a", "1"), ("b", "2")]);
        let l2 = Labels::from_pairs([("b", "2"), ("a", "1")]); // different insertion order
        assert_eq!(l1.fingerprint(), l2.fingerprint());
    }

    #[test]
    fn test_fingerprint_differs_for_different_labels() {
        let l1 = Labels::from_pairs([("__name__", "cpu")]);
        let l2 = Labels::from_pairs([("__name__", "memory")]);
        assert_ne!(l1.fingerprint(), l2.fingerprint());
    }

    // ── upstream: TestLabelsString (Prometheus labels_test.go) ───────────────

    #[test]
    fn test_selector_excludes_name() {
        let l = Labels::from_pairs([("__name__", "cpu"), ("job", "prometheus"), ("instance", "localhost:9090")]);
        let sel = l.to_selector();
        assert!(!sel.contains("__name__"), "selector must not contain __name__");
        assert!(sel.contains("job=\"prometheus\""));
        assert!(sel.contains("instance=\"localhost:9090\""));
    }

    // ── upstream: TestLabelsGet ───────────────────────────────────────────────

    #[test]
    fn test_get_present() {
        let l = Labels::from_pairs([("env", "prod")]);
        assert_eq!(l.get("env"), Some("prod"));
    }

    #[test]
    fn test_get_absent() {
        let l = Labels::from_pairs([("env", "prod")]);
        assert_eq!(l.get("missing"), None);
    }

    // ── upstream: TestLabelsInsertRemove ─────────────────────────────────────

    #[test]
    fn test_insert_and_remove() {
        let mut l = Labels::new();
        l.insert("k", "v");
        assert_eq!(l.get("k"), Some("v"));
        l.remove("k");
        assert_eq!(l.get("k"), None);
    }

    // ── upstream: TestLabelsWithOnly / TestLabelsWithout ──────────────────────

    #[test]
    fn test_with_only() {
        let l = Labels::from_pairs([("a", "1"), ("b", "2"), ("c", "3")]);
        let sub = l.with_only(&["a", "c"]);
        assert_eq!(sub.get("a"), Some("1"));
        assert_eq!(sub.get("c"), Some("3"));
        assert_eq!(sub.get("b"), None);
    }

    #[test]
    fn test_without() {
        let l = Labels::from_pairs([("a", "1"), ("b", "2"), ("c", "3")]);
        let sub = l.without(&["b"]);
        assert_eq!(sub.get("a"), Some("1"));
        assert_eq!(sub.get("b"), None);
        assert_eq!(sub.get("c"), Some("3"));
    }

    // ── upstream: TestLabelsSerialization ────────────────────────────────────

    #[test]
    fn test_roundtrip_json() {
        let l = Labels::from_pairs([("__name__", "http_requests_total"), ("method", "GET")]);
        let json = serde_json::to_string(&l).unwrap();
        let l2: Labels = serde_json::from_str(&json).unwrap();
        assert_eq!(l, l2);
    }

    // ── upstream: TestLabelsEmpty ─────────────────────────────────────────────

    #[test]
    fn test_empty() {
        let l = Labels::new();
        assert!(l.is_empty());
        assert_eq!(l.len(), 0);
    }

    // ── upstream: TestLabelsMetricName ────────────────────────────────────────

    #[test]
    fn test_metric_name() {
        let l = Labels::from_pairs([("__name__", "requests")]);
        assert_eq!(l.metric_name(), Some("requests"));
        let l2 = Labels::new();
        assert_eq!(l2.metric_name(), None);
    }
}

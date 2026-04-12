//! Core Prometheus data model.

#![allow(dead_code)]

use std::collections::BTreeMap;
use serde::{Deserialize, Serialize};
use regex::Regex;

/// Milliseconds since Unix epoch.
pub type Timestamp = i64;

/// Floating-point sample value.
pub type Value = f64;

/// A set of labels identifying a time series.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default)]
pub struct Labels(pub BTreeMap<String, String>);

impl Labels {
    /// FNV-1a hash over all label key=value pairs.
    pub fn fingerprint(&self) -> u64 {
        const FNV_OFFSET: u64 = 14_695_981_039_346_656_037;
        const FNV_PRIME: u64 = 1_099_511_628_211;
        let mut hash = FNV_OFFSET;
        for (k, v) in &self.0 {
            for byte in k.bytes() {
                hash ^= byte as u64;
                hash = hash.wrapping_mul(FNV_PRIME);
            }
            hash ^= b'=' as u64;
            hash = hash.wrapping_mul(FNV_PRIME);
            for byte in v.bytes() {
                hash ^= byte as u64;
                hash = hash.wrapping_mul(FNV_PRIME);
            }
            hash ^= b',' as u64;
            hash = hash.wrapping_mul(FNV_PRIME);
        }
        hash
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(|s| s.as_str())
    }

    pub fn metric_name(&self) -> Option<&str> {
        self.get("__name__")
    }

    /// Return a new Labels with an additional key=value pair.
    pub fn with(&self, key: impl Into<String>, value: impl Into<String>) -> Self {
        let mut m = self.0.clone();
        m.insert(key.into(), value.into());
        Labels(m)
    }

    /// Build Labels from an iterator of (key, value) pairs.
    pub fn from_pairs<I, K, V>(pairs: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        Labels(pairs.into_iter().map(|(k, v)| (k.into(), v.into())).collect())
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MetricType {
    Counter,
    Gauge,
    Histogram,
    Summary,
    Untyped,
}

/// A single (timestamp, value) observation.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Sample {
    pub timestamp: Timestamp,
    pub value: Value,
}

impl Sample {
    pub fn new(timestamp: Timestamp, value: Value) -> Self {
        Self { timestamp, value }
    }
}

/// A complete time series: labels + ordered samples.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeSeries {
    pub labels: Labels,
    pub samples: Vec<Sample>,
}

impl TimeSeries {
    pub fn new(labels: Labels, samples: Vec<Sample>) -> Self {
        Self { labels, samples }
    }
}

/// Matchers used for series selection.
#[derive(Debug, Clone)]
pub enum LabelMatcher {
    Equal { name: String, value: String },
    NotEqual { name: String, value: String },
    RegexMatch { name: String, pattern: String },
    RegexNotMatch { name: String, pattern: String },
}

impl LabelMatcher {
    pub fn matches(&self, labels: &Labels) -> bool {
        match self {
            LabelMatcher::Equal { name, value } => {
                labels.get(name).unwrap_or("") == value.as_str()
            }
            LabelMatcher::NotEqual { name, value } => {
                labels.get(name).unwrap_or("") != value.as_str()
            }
            LabelMatcher::RegexMatch { name, pattern } => {
                let full = format!("^(?:{})$", pattern);
                Regex::new(&full)
                    .map(|re| re.is_match(labels.get(name).unwrap_or("")))
                    .unwrap_or(false)
            }
            LabelMatcher::RegexNotMatch { name, pattern } => {
                let full = format!("^(?:{})$", pattern);
                Regex::new(&full)
                    .map(|re| !re.is_match(labels.get(name).unwrap_or("")))
                    .unwrap_or(true)
            }
        }
    }

    pub fn name(&self) -> &str {
        match self {
            LabelMatcher::Equal { name, .. } => name,
            LabelMatcher::NotEqual { name, .. } => name,
            LabelMatcher::RegexMatch { name, .. } => name,
            LabelMatcher::RegexNotMatch { name, .. } => name,
        }
    }
}

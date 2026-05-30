// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Core Prometheus data model: labels, samples, time-series, matchers.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A set of key=value label pairs identifying a time series.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default)]
pub struct Labels(pub BTreeMap<String, String>);

impl Labels {
    pub fn new() -> Self {
        Self(BTreeMap::new())
    }

    pub fn from_pairs(
        pairs: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
    ) -> Self {
        Self(
            pairs
                .into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        )
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.get(name).map(|s| s.as_str())
    }

    pub fn insert(&mut self, name: impl Into<String>, value: impl Into<String>) {
        self.0.insert(name.into(), value.into());
    }

    pub fn metric_name(&self) -> Option<&str> {
        self.get("__name__")
    }

    /// FNV-1a fingerprint for O(1) identity.
    pub fn fingerprint(&self) -> u64 {
        let mut hash: u64 = 14_695_981_039_346_656_037;
        for (k, v) in &self.0 {
            for byte in k
                .bytes()
                .chain(std::iter::once(b'='))
                .chain(v.bytes())
                .chain(std::iter::once(b','))
            {
                hash ^= byte as u64;
                hash = hash.wrapping_mul(1_099_511_628_211);
            }
        }
        hash
    }

    /// Return a copy without the __name__ label (for grouping).
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

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }
}

impl std::fmt::Display for Labels {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{{")?;
        let mut first = true;
        for (k, v) in &self.0 {
            if k == "__name__" {
                continue;
            }
            if !first {
                write!(f, ",")?;
            }
            write!(f, "{}=\"{}\"", k, v)?;
            first = false;
        }
        write!(f, "}}")
    }
}

/// A single (timestamp_ms, value) observation.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Sample {
    pub timestamp_ms: i64,
    pub value: f64,
}

impl Sample {
    pub fn new(timestamp_ms: i64, value: f64) -> Self {
        Self {
            timestamp_ms,
            value,
        }
    }
}

/// A labelled time series: metadata + ordered samples.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeSeries {
    pub labels: Labels,
    pub samples: Vec<Sample>,
}

impl TimeSeries {
    pub fn new(labels: Labels) -> Self {
        Self {
            labels,
            samples: Vec::new(),
        }
    }

    pub fn push(&mut self, sample: Sample) {
        self.samples.push(sample);
    }
}

/// Label matcher operator.
#[derive(Debug, Clone, PartialEq)]
pub enum MatchOp {
    Equal,
    NotEqual,
    RegexMatch,
    RegexNotMatch,
}

/// A single label matcher used in vector selectors.
#[derive(Debug, Clone)]
pub struct LabelMatcher {
    pub name: String,
    pub op: MatchOp,
    pub value: String,
    regex: Option<regex::Regex>,
}

impl LabelMatcher {
    pub fn equal(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            op: MatchOp::Equal,
            value: value.into(),
            regex: None,
        }
    }

    pub fn not_equal(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            op: MatchOp::NotEqual,
            value: value.into(),
            regex: None,
        }
    }

    pub fn regex(
        name: impl Into<String>,
        pattern: impl Into<String>,
    ) -> crate::error::Result<Self> {
        let value: String = pattern.into();
        let anchored = format!("^(?:{})$", value);
        let re = regex::Regex::new(&anchored)
            .map_err(|e| crate::error::MetricsError::Parse(e.to_string()))?;
        Ok(Self {
            name: name.into(),
            op: MatchOp::RegexMatch,
            value,
            regex: Some(re),
        })
    }

    pub fn not_regex(
        name: impl Into<String>,
        pattern: impl Into<String>,
    ) -> crate::error::Result<Self> {
        let value: String = pattern.into();
        let anchored = format!("^(?:{})$", value);
        let re = regex::Regex::new(&anchored)
            .map_err(|e| crate::error::MetricsError::Parse(e.to_string()))?;
        Ok(Self {
            name: name.into(),
            op: MatchOp::RegexNotMatch,
            value,
            regex: Some(re),
        })
    }

    pub fn matches(&self, labels: &Labels) -> bool {
        self.matches_value(labels.get(&self.name).unwrap_or(""))
    }

    /// Evaluate the matcher against a single raw label value (an absent label is
    /// passed as `""`). This is the primitive the TSDB inverted index uses when
    /// resolving postings per Prometheus `PostingsForMatchers`.
    pub fn matches_value(&self, value: &str) -> bool {
        match self.op {
            MatchOp::Equal => value == self.value,
            MatchOp::NotEqual => value != self.value,
            MatchOp::RegexMatch => self
                .regex
                .as_ref()
                .map(|r| r.is_match(value))
                .unwrap_or(false),
            MatchOp::RegexNotMatch => !self
                .regex
                .as_ref()
                .map(|r| r.is_match(value))
                .unwrap_or(false),
        }
    }

    /// Whether this matcher matches the empty string. Per Prometheus, such a
    /// matcher also selects every series that does not carry the label at all.
    pub fn matches_empty(&self) -> bool {
        self.matches_value("")
    }
}

impl PartialEq for LabelMatcher {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name && self.op == other.op && self.value == other.value
    }
}

/// Prometheus metric type.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub enum MetricType {
    #[default]
    Untyped,
    Counter,
    Gauge,
    Histogram,
    Summary,
    GaugeHistogram,
    Info,
    StateSet,
}

impl std::str::FromStr for MetricType {
    type Err = ();
    fn from_str(s: &str) -> std::result::Result<Self, ()> {
        Ok(match s.to_ascii_lowercase().as_str() {
            "counter" => Self::Counter,
            "gauge" => Self::Gauge,
            "histogram" => Self::Histogram,
            "summary" => Self::Summary,
            "gaugehistogram" => Self::GaugeHistogram,
            "info" => Self::Info,
            "stateset" => Self::StateSet,
            _ => Self::Untyped,
        })
    }
}

/// Instant vector: one sample per series at a fixed timestamp.
pub type InstantVector = Vec<(Labels, f64)>;

/// Range vector: a window of samples per series.
pub type RangeVector = Vec<(Labels, Vec<Sample>)>;

/// PromQL evaluation result.
#[derive(Debug, Clone)]
pub enum QueryResult {
    Scalar(f64),
    String(String),
    InstantVector(InstantVector),
    RangeVector(RangeVector),
}

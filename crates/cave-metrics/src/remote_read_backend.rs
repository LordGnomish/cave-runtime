// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Remote-read backend trait + in-memory backend.
//!
//! upstream: prometheus/prometheus — storage/remote (read path)
//!
//! Upstream defines a `Queryable` interface for remote read where a
//! Prometheus instance asks a long-term store for samples in a label
//! matcher window. We port the same interface as a `RemoteReadBackend`
//! trait + one in-memory backend that other crates (cave-cache,
//! cave-lakehouse) can wire on top of their persistent storage.

use std::collections::BTreeMap;

#[derive(Default, Debug, Clone, PartialEq)]
pub struct Sample {
    pub timestamp_ms: i64,
    pub value: f64,
}

#[derive(Default, Debug, Clone, PartialEq)]
pub struct SeriesSamples {
    pub labels: BTreeMap<String, String>,
    pub samples: Vec<Sample>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MatcherKind {
    Equal,
    NotEqual,
    RegexMatch,
    RegexNoMatch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelMatcher {
    pub kind: MatcherKind,
    pub name: String,
    pub value: String,
}

impl LabelMatcher {
    pub fn eq(name: &str, value: &str) -> Self {
        Self {
            kind: MatcherKind::Equal,
            name: name.to_string(),
            value: value.to_string(),
        }
    }
    pub fn ne(name: &str, value: &str) -> Self {
        Self {
            kind: MatcherKind::NotEqual,
            name: name.to_string(),
            value: value.to_string(),
        }
    }
    pub fn re(name: &str, value: &str) -> Self {
        Self {
            kind: MatcherKind::RegexMatch,
            name: name.to_string(),
            value: value.to_string(),
        }
    }
    pub fn rne(name: &str, value: &str) -> Self {
        Self {
            kind: MatcherKind::RegexNoMatch,
            name: name.to_string(),
            value: value.to_string(),
        }
    }

    pub fn matches(&self, labels: &BTreeMap<String, String>) -> bool {
        let v = labels.get(&self.name).map(String::as_str).unwrap_or("");
        match self.kind {
            MatcherKind::Equal => v == self.value,
            MatcherKind::NotEqual => v != self.value,
            MatcherKind::RegexMatch => glob_match(&self.value, v),
            MatcherKind::RegexNoMatch => !glob_match(&self.value, v),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ReadQuery {
    pub start_ms: i64,
    pub end_ms: i64,
    pub matchers: Vec<LabelMatcher>,
}

/// Trait other crates implement to plug into Prometheus's remote-read.
pub trait RemoteReadBackend: Send + Sync {
    fn read(&self, q: &ReadQuery) -> Vec<SeriesSamples>;
    fn name(&self) -> &'static str;
}

// ─── In-memory backend ──────────────────────────────────────────────────

#[derive(Default, Debug, Clone)]
pub struct MemoryReadBackend {
    series: Vec<SeriesSamples>,
}

impl MemoryReadBackend {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_series(&mut self, labels: BTreeMap<String, String>, samples: Vec<Sample>) {
        self.series.push(SeriesSamples { labels, samples });
    }

    pub fn series_count(&self) -> usize {
        self.series.len()
    }
}

impl RemoteReadBackend for MemoryReadBackend {
    fn name(&self) -> &'static str {
        "memory"
    }
    fn read(&self, q: &ReadQuery) -> Vec<SeriesSamples> {
        let mut out = Vec::new();
        for s in &self.series {
            if q.matchers.iter().all(|m| m.matches(&s.labels)) {
                let samples: Vec<Sample> = s
                    .samples
                    .iter()
                    .filter(|sa| sa.timestamp_ms >= q.start_ms && sa.timestamp_ms <= q.end_ms)
                    .cloned()
                    .collect();
                if !samples.is_empty() {
                    out.push(SeriesSamples {
                        labels: s.labels.clone(),
                        samples,
                    });
                }
            }
        }
        out
    }
}

/// Lightweight glob-style matcher used by [`MatcherKind::RegexMatch`] —
/// supports `.*` (zero+), `.+` (one+), `?` (single), and `|` alternation.
/// Sufficient for Prometheus's bounded set of recording-rule regexes.
fn glob_match(pattern: &str, text: &str) -> bool {
    for alt in pattern.split('|') {
        if simple_glob(alt, text) {
            return true;
        }
    }
    false
}

fn simple_glob(pattern: &str, text: &str) -> bool {
    let pat: Vec<char> = pattern.chars().collect();
    let txt: Vec<char> = text.chars().collect();
    glob_rec(&pat, &txt)
}

fn glob_rec(pat: &[char], txt: &[char]) -> bool {
    if pat.is_empty() {
        return txt.is_empty();
    }
    // .* — zero or more
    if pat.len() >= 2 && pat[0] == '.' && pat[1] == '*' {
        for i in 0..=txt.len() {
            if glob_rec(&pat[2..], &txt[i..]) {
                return true;
            }
        }
        return false;
    }
    // .+ — one or more
    if pat.len() >= 2 && pat[0] == '.' && pat[1] == '+' {
        for i in 1..=txt.len() {
            if glob_rec(&pat[2..], &txt[i..]) {
                return true;
            }
        }
        return false;
    }
    // ? — single
    if pat[0] == '?' && !txt.is_empty() {
        return glob_rec(&pat[1..], &txt[1..]);
    }
    // . — single
    if pat[0] == '.' && !txt.is_empty() {
        return glob_rec(&pat[1..], &txt[1..]);
    }
    if !txt.is_empty() && pat[0] == txt[0] {
        return glob_rec(&pat[1..], &txt[1..]);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn s(ts: i64, v: f64) -> Sample {
        Sample {
            timestamp_ms: ts,
            value: v,
        }
    }

    fn backend_with_demo() -> MemoryReadBackend {
        let mut b = MemoryReadBackend::new();
        b.add_series(
            labels(&[("__name__", "up"), ("job", "api")]),
            vec![s(1_000, 1.0), s(2_000, 1.0), s(3_000, 0.0)],
        );
        b.add_series(
            labels(&[("__name__", "up"), ("job", "db")]),
            vec![s(1_000, 1.0), s(2_000, 1.0)],
        );
        b.add_series(
            labels(&[("__name__", "latency"), ("job", "api")]),
            vec![s(1_000, 0.5)],
        );
        b
    }

    #[test]
    fn equal_matcher_returns_only_matching_series() {
        let b = backend_with_demo();
        let q = ReadQuery {
            start_ms: 0,
            end_ms: 10_000,
            matchers: vec![LabelMatcher::eq("__name__", "up")],
        };
        let res = b.read(&q);
        assert_eq!(res.len(), 2);
    }

    #[test]
    fn multi_matcher_and_semantics() {
        let b = backend_with_demo();
        let q = ReadQuery {
            start_ms: 0,
            end_ms: 10_000,
            matchers: vec![
                LabelMatcher::eq("__name__", "up"),
                LabelMatcher::eq("job", "api"),
            ],
        };
        let res = b.read(&q);
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].labels["job"], "api");
    }

    #[test]
    fn time_window_filters_samples() {
        let b = backend_with_demo();
        let q = ReadQuery {
            start_ms: 1_500,
            end_ms: 2_500,
            matchers: vec![
                LabelMatcher::eq("__name__", "up"),
                LabelMatcher::eq("job", "api"),
            ],
        };
        let res = b.read(&q);
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].samples.len(), 1);
        assert_eq!(res[0].samples[0].timestamp_ms, 2_000);
    }

    #[test]
    fn regex_match_dot_star() {
        let b = backend_with_demo();
        let q = ReadQuery {
            start_ms: 0,
            end_ms: 10_000,
            matchers: vec![LabelMatcher::re("__name__", "lat.*")],
        };
        let res = b.read(&q);
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].labels["__name__"], "latency");
    }

    #[test]
    fn not_equal_excludes_match() {
        let b = backend_with_demo();
        let q = ReadQuery {
            start_ms: 0,
            end_ms: 10_000,
            matchers: vec![
                LabelMatcher::eq("__name__", "up"),
                LabelMatcher::ne("job", "api"),
            ],
        };
        let res = b.read(&q);
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].labels["job"], "db");
    }

    #[test]
    fn regex_alternation_works() {
        let b = backend_with_demo();
        let q = ReadQuery {
            start_ms: 0,
            end_ms: 10_000,
            matchers: vec![LabelMatcher::re("job", "api|db")],
        };
        let res = b.read(&q);
        assert_eq!(res.len(), 3);
    }

    #[test]
    fn empty_window_returns_no_series() {
        let b = backend_with_demo();
        let q = ReadQuery {
            start_ms: 10_000,
            end_ms: 20_000,
            matchers: vec![LabelMatcher::eq("__name__", "up")],
        };
        let res = b.read(&q);
        assert!(res.is_empty());
    }

    #[test]
    fn backend_name_is_memory() {
        let b = MemoryReadBackend::new();
        assert_eq!(b.name(), "memory");
    }
}

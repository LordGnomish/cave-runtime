// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Cost Insight plugin — per-tenant spend breakdown and trend.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CostEntry {
    pub tenant: String,
    pub day: String,
    pub category: String,
    pub usd: f64,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CostError {
    #[error("invalid amount {0}")]
    InvalidAmount(String),
    #[error("invalid day {0}")]
    InvalidDay(String),
    #[error("category empty")]
    EmptyCategory,
}

/// Strict ISO `YYYY-MM-DD` format: digits at positions 0..=3, 5..=6, 8..=9 and
/// dashes at positions 4 and 7.
fn is_iso_day(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.len() != 10 {
        return false;
    }
    if bytes[4] != b'-' || bytes[7] != b'-' {
        return false;
    }
    for &i in &[0_usize, 1, 2, 3, 5, 6, 8, 9] {
        if !bytes[i].is_ascii_digit() {
            return false;
        }
    }
    true
}

#[derive(Debug, Default)]
pub struct CostInsightPlugin {
    entries: Vec<CostEntry>,
}

impl CostInsightPlugin {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&mut self, e: CostEntry) -> Result<(), CostError> {
        if e.usd.is_nan() || e.usd.is_infinite() || e.usd < 0.0 {
            return Err(CostError::InvalidAmount(format!("{}", e.usd)));
        }
        if !is_iso_day(&e.day) {
            return Err(CostError::InvalidDay(e.day));
        }
        if e.category.is_empty() {
            return Err(CostError::EmptyCategory);
        }
        self.entries.push(e);
        Ok(())
    }

    pub fn total(&self, tenant: &str) -> f64 {
        self.entries.iter().filter(|e| e.tenant == tenant).map(|e| e.usd).sum()
    }

    pub fn by_category(&self, tenant: &str) -> HashMap<String, f64> {
        let mut acc: HashMap<String, f64> = HashMap::new();
        for e in self.entries.iter().filter(|e| e.tenant == tenant) {
            *acc.entry(e.category.clone()).or_insert(0.0) += e.usd;
        }
        acc
    }

    pub fn by_day(&self, tenant: &str) -> Vec<(String, f64)> {
        let mut acc: HashMap<String, f64> = HashMap::new();
        for e in self.entries.iter().filter(|e| e.tenant == tenant) {
            *acc.entry(e.day.clone()).or_insert(0.0) += e.usd;
        }
        let mut out: Vec<(String, f64)> = acc.into_iter().collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    pub fn forecast_next_day(&self, tenant: &str) -> f64 {
        let series = self.by_day(tenant);
        let n = series.len();
        if n == 0 {
            return 0.0;
        }
        // Use last 7 days average, or full series if shorter.
        let take = n.min(7);
        let recent: Vec<f64> = series.iter().rev().take(take).map(|(_, v)| *v).collect();
        recent.iter().sum::<f64>() / recent.len() as f64
    }

    pub fn count(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(t: &str, d: &str, c: &str, u: f64) -> CostEntry {
        CostEntry { tenant: t.into(), day: d.into(), category: c.into(), usd: u }
    }

    #[test]
    fn record_valid() {
        let mut p = CostInsightPlugin::new();
        p.record(entry("acme", "2026-04-26", "compute", 12.5)).unwrap();
        assert_eq!(p.count(), 1);
    }

    #[test]
    fn record_negative_rejected() {
        let mut p = CostInsightPlugin::new();
        let err = p.record(entry("acme", "2026-04-26", "compute", -1.0)).unwrap_err();
        assert!(matches!(err, CostError::InvalidAmount(_)));
    }

    #[test]
    fn record_nan_rejected() {
        let mut p = CostInsightPlugin::new();
        let err = p.record(entry("acme", "2026-04-26", "compute", f64::NAN)).unwrap_err();
        assert!(matches!(err, CostError::InvalidAmount(_)));
    }

    #[test]
    fn record_infinite_rejected() {
        let mut p = CostInsightPlugin::new();
        let err = p.record(entry("acme", "2026-04-26", "compute", f64::INFINITY)).unwrap_err();
        assert!(matches!(err, CostError::InvalidAmount(_)));
    }

    #[test]
    fn record_invalid_day_format_rejected() {
        let mut p = CostInsightPlugin::new();
        let err = p.record(entry("acme", "26-04-2026", "x", 1.0)).unwrap_err();
        assert!(matches!(err, CostError::InvalidDay(_)));
    }

    #[test]
    fn record_invalid_day_chars_rejected() {
        let mut p = CostInsightPlugin::new();
        let err = p.record(entry("acme", "2026/04/26", "x", 1.0)).unwrap_err();
        assert!(matches!(err, CostError::InvalidDay(_)));
    }

    #[test]
    fn record_empty_category_rejected() {
        let mut p = CostInsightPlugin::new();
        let err = p.record(entry("acme", "2026-04-26", "", 1.0)).unwrap_err();
        assert!(matches!(err, CostError::EmptyCategory));
    }

    #[test]
    fn total_sums_for_tenant() {
        let mut p = CostInsightPlugin::new();
        p.record(entry("acme", "2026-04-26", "compute", 10.0)).unwrap();
        p.record(entry("acme", "2026-04-26", "storage", 5.0)).unwrap();
        p.record(entry("globex", "2026-04-26", "compute", 100.0)).unwrap();
        assert_eq!(p.total("acme"), 15.0);
    }

    #[test]
    fn by_category_sums_per_category() {
        let mut p = CostInsightPlugin::new();
        p.record(entry("acme", "2026-04-26", "compute", 10.0)).unwrap();
        p.record(entry("acme", "2026-04-27", "compute", 12.0)).unwrap();
        p.record(entry("acme", "2026-04-26", "storage", 5.0)).unwrap();
        let by = p.by_category("acme");
        assert_eq!(by["compute"], 22.0);
        assert_eq!(by["storage"], 5.0);
    }

    #[test]
    fn by_day_sorted_ascending() {
        let mut p = CostInsightPlugin::new();
        p.record(entry("acme", "2026-04-27", "x", 1.0)).unwrap();
        p.record(entry("acme", "2026-04-26", "x", 2.0)).unwrap();
        p.record(entry("acme", "2026-04-28", "x", 3.0)).unwrap();
        let series = p.by_day("acme");
        let days: Vec<&str> = series.iter().map(|(d, _)| d.as_str()).collect();
        assert_eq!(days, vec!["2026-04-26", "2026-04-27", "2026-04-28"]);
    }

    #[test]
    fn by_day_sums_within_day() {
        let mut p = CostInsightPlugin::new();
        p.record(entry("acme", "2026-04-26", "compute", 10.0)).unwrap();
        p.record(entry("acme", "2026-04-26", "storage", 5.0)).unwrap();
        let series = p.by_day("acme");
        assert_eq!(series.len(), 1);
        assert_eq!(series[0].1, 15.0);
    }

    #[test]
    fn forecast_zero_when_empty() {
        let p = CostInsightPlugin::new();
        assert_eq!(p.forecast_next_day("acme"), 0.0);
    }

    #[test]
    fn forecast_uses_last_7_days() {
        let mut p = CostInsightPlugin::new();
        for d in 1..=10u32 {
            let day = format!("2026-04-{:02}", d);
            p.record(entry("acme", &day, "x", d as f64)).unwrap();
        }
        let fc = p.forecast_next_day("acme");
        // average of days 4..10 = 4..10 sum / 7
        let expected: f64 = (4..=10).sum::<u32>() as f64 / 7.0;
        assert!((fc - expected).abs() < 1e-9);
    }

    #[test]
    fn forecast_uses_full_when_shorter_than_7() {
        let mut p = CostInsightPlugin::new();
        p.record(entry("acme", "2026-04-26", "x", 4.0)).unwrap();
        p.record(entry("acme", "2026-04-27", "x", 8.0)).unwrap();
        let fc = p.forecast_next_day("acme");
        assert!((fc - 6.0).abs() < 1e-9);
    }

    #[test]
    fn count_tracks_records() {
        let mut p = CostInsightPlugin::new();
        for _ in 0..3 {
            p.record(entry("acme", "2026-04-26", "x", 1.0)).unwrap();
        }
        assert_eq!(p.count(), 3);
    }

    #[test]
    fn cost_entry_serializes() {
        let e = entry("acme", "2026-04-26", "compute", 12.5);
        let s = serde_json::to_string(&e).unwrap();
        assert!(s.contains("\"usd\":12.5"));
    }
}

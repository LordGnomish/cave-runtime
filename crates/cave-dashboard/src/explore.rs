// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Explore — multi-source, log+metric correlation panel.
//!
//! upstream: grafana/grafana — public/app/features/explore
//!
//! Grafana's Explore is an ad-hoc query workbench that lets users
//! cross-reference data from multiple datasources at once (e.g.
//! Prometheus metric ↔ Loki log entries ↔ Tempo trace). We port the
//! controller-side breadth: the multi-datasource query plan, the
//! correlation rules (split-by-time, split-by-trace-id), and the
//! result merge into a single ExploreFrame the portal can render.

use std::collections::BTreeMap;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceKind {
    Prometheus,
    Loki,
    Jaeger,
    Tempo,
    Postgres,
    Elasticsearch,
    InfluxDB,
}

#[derive(Debug, Clone)]
pub struct ExploreQuery {
    pub ref_id: String,
    pub datasource: String,
    pub kind: SourceKind,
    pub expr: String,
    pub from_unix_ms: i64,
    pub to_unix_ms: i64,
    pub max_data_points: u32,
}

#[derive(Debug, Clone)]
pub struct ExploreFrame {
    pub ref_id: String,
    pub kind: SourceKind,
    pub points: Vec<ExplorePoint>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExplorePoint {
    pub timestamp_ms: i64,
    pub value: Option<f64>,
    pub log_line: Option<String>,
    pub trace_id: Option<String>,
    pub labels: BTreeMap<String, String>,
}

/// One correlation rule — the rule fires when both sides match.
#[derive(Debug, Clone)]
pub enum CorrelationRule {
    /// Frames are correlated by the trace id label.
    ByTraceId,
    /// Frames are correlated by exact timestamp.
    ByExactTimestamp,
    /// Frames are correlated by sliding time window in ms.
    ByTimeWindow { window_ms: i64 },
}

#[derive(Default)]
pub struct ExploreController {
    pub queries: Vec<ExploreQuery>,
    pub correlations: Vec<CorrelationRule>,
}

impl ExploreController {
    pub fn new() -> Self { Self::default() }

    pub fn push(mut self, q: ExploreQuery) -> Self {
        self.queries.push(q);
        self
    }

    pub fn with_correlation(mut self, rule: CorrelationRule) -> Self {
        self.correlations.push(rule);
        self
    }

    /// Plan the queries by datasource. Returns a map of `datasource → ref_ids`
    /// so the runtime can issue one batched request per back-end.
    pub fn plan(&self) -> HashMap<String, Vec<String>> {
        let mut groups: HashMap<String, Vec<String>> = HashMap::new();
        for q in &self.queries {
            groups.entry(q.datasource.clone()).or_default().push(q.ref_id.clone());
        }
        groups
    }

    /// Correlate the returned frames against the active rules. The
    /// resulting `CorrelatedRow` is the cross-source pivot the UI shows.
    pub fn correlate(&self, frames: &[ExploreFrame]) -> Vec<CorrelatedRow> {
        let mut rows: Vec<CorrelatedRow> = Vec::new();
        for rule in &self.correlations {
            match rule {
                CorrelationRule::ByTraceId => {
                    let mut by_trace: HashMap<String, CorrelatedRow> = HashMap::new();
                    for f in frames {
                        for p in &f.points {
                            if let Some(tid) = &p.trace_id {
                                let row = by_trace.entry(tid.clone()).or_insert_with(|| CorrelatedRow {
                                    pivot: format!("trace={}", tid),
                                    points: Vec::new(),
                                });
                                row.points.push((f.ref_id.clone(), p.clone()));
                            }
                        }
                    }
                    rows.extend(by_trace.into_values());
                }
                CorrelationRule::ByExactTimestamp => {
                    let mut by_ts: HashMap<i64, CorrelatedRow> = HashMap::new();
                    for f in frames {
                        for p in &f.points {
                            let row = by_ts.entry(p.timestamp_ms).or_insert_with(|| CorrelatedRow {
                                pivot: format!("ts={}", p.timestamp_ms),
                                points: Vec::new(),
                            });
                            row.points.push((f.ref_id.clone(), p.clone()));
                        }
                    }
                    rows.extend(by_ts.into_values());
                }
                CorrelationRule::ByTimeWindow { window_ms } => {
                    let mut all: Vec<(String, ExplorePoint)> = Vec::new();
                    for f in frames {
                        for p in &f.points {
                            all.push((f.ref_id.clone(), p.clone()));
                        }
                    }
                    all.sort_by_key(|(_, p)| p.timestamp_ms);
                    let mut current_bucket_start: Option<i64> = None;
                    let mut bucket: Vec<(String, ExplorePoint)> = Vec::new();
                    for entry in all {
                        let starts_new_bucket = match current_bucket_start {
                            None => true,
                            Some(start) => entry.1.timestamp_ms.saturating_sub(start) > *window_ms,
                        };
                        if starts_new_bucket {
                            if let Some(start) = current_bucket_start.take() {
                                if !bucket.is_empty() {
                                    rows.push(CorrelatedRow {
                                        pivot: format!("window={}", start),
                                        points: std::mem::take(&mut bucket),
                                    });
                                }
                            }
                            current_bucket_start = Some(entry.1.timestamp_ms);
                        }
                        bucket.push(entry);
                    }
                    if let Some(start) = current_bucket_start {
                        if !bucket.is_empty() {
                            rows.push(CorrelatedRow {
                                pivot: format!("window={}", start),
                                points: bucket,
                            });
                        }
                    }
                }
            }
        }
        rows.sort_by(|a, b| a.pivot.cmp(&b.pivot));
        rows
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CorrelatedRow {
    pub pivot: String,
    pub points: Vec<(String, ExplorePoint)>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metric_point(ts: i64, v: f64) -> ExplorePoint {
        ExplorePoint {
            timestamp_ms: ts,
            value: Some(v),
            log_line: None,
            trace_id: None,
            labels: BTreeMap::new(),
        }
    }

    fn log_point(ts: i64, line: &str, trace: Option<&str>) -> ExplorePoint {
        ExplorePoint {
            timestamp_ms: ts,
            value: None,
            log_line: Some(line.to_string()),
            trace_id: trace.map(str::to_string),
            labels: BTreeMap::new(),
        }
    }

    fn query(ref_id: &str, ds: &str, kind: SourceKind) -> ExploreQuery {
        ExploreQuery {
            ref_id: ref_id.into(),
            datasource: ds.into(),
            kind,
            expr: "up".into(),
            from_unix_ms: 0,
            to_unix_ms: 10_000,
            max_data_points: 100,
        }
    }

    #[test]
    fn plan_groups_queries_by_datasource() {
        let ctrl = ExploreController::new()
            .push(query("A", "prom", SourceKind::Prometheus))
            .push(query("B", "prom", SourceKind::Prometheus))
            .push(query("C", "loki", SourceKind::Loki));
        let plan = ctrl.plan();
        assert_eq!(plan.len(), 2);
        let mut prom = plan["prom"].clone();
        prom.sort();
        assert_eq!(prom, vec!["A".to_string(), "B".to_string()]);
        assert_eq!(plan["loki"], vec!["C".to_string()]);
    }

    #[test]
    fn correlate_by_trace_id_pivots_across_sources() {
        let ctrl = ExploreController::new().with_correlation(CorrelationRule::ByTraceId);
        let frames = vec![
            ExploreFrame {
                ref_id: "A".into(),
                kind: SourceKind::Loki,
                points: vec![log_point(1, "err", Some("t1"))],
            },
            ExploreFrame {
                ref_id: "B".into(),
                kind: SourceKind::Tempo,
                points: vec![ExplorePoint {
                    timestamp_ms: 1,
                    value: Some(0.4),
                    log_line: None,
                    trace_id: Some("t1".into()),
                    labels: BTreeMap::new(),
                }],
            },
        ];
        let rows = ctrl.correlate(&frames);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].pivot, "trace=t1");
        assert_eq!(rows[0].points.len(), 2);
    }

    #[test]
    fn correlate_by_exact_timestamp_groups_same_ts() {
        let ctrl = ExploreController::new().with_correlation(CorrelationRule::ByExactTimestamp);
        let frames = vec![
            ExploreFrame {
                ref_id: "A".into(),
                kind: SourceKind::Prometheus,
                points: vec![metric_point(100, 1.0), metric_point(200, 2.0)],
            },
            ExploreFrame {
                ref_id: "B".into(),
                kind: SourceKind::Loki,
                points: vec![log_point(100, "ok", None)],
            },
        ];
        let rows = ctrl.correlate(&frames);
        let row_at_100 = rows.iter().find(|r| r.pivot == "ts=100").unwrap();
        assert_eq!(row_at_100.points.len(), 2);
    }

    #[test]
    fn correlate_by_time_window_buckets_within_threshold() {
        let ctrl = ExploreController::new()
            .with_correlation(CorrelationRule::ByTimeWindow { window_ms: 50 });
        let frames = vec![ExploreFrame {
            ref_id: "A".into(),
            kind: SourceKind::Prometheus,
            points: vec![
                metric_point(0, 0.1),
                metric_point(40, 0.2),
                metric_point(100, 0.3),
            ],
        }];
        let rows = ctrl.correlate(&frames);
        // First bucket holds ts=0,40 (within 50ms); second holds ts=100
        assert_eq!(rows.len(), 2);
        assert_eq!(rows.iter().filter(|r| r.points.len() == 2).count(), 1);
        assert_eq!(rows.iter().filter(|r| r.points.len() == 1).count(), 1);
    }

    #[test]
    fn correlate_no_rule_returns_no_rows() {
        let ctrl = ExploreController::new();
        let frames = vec![ExploreFrame {
            ref_id: "A".into(),
            kind: SourceKind::Prometheus,
            points: vec![metric_point(0, 1.0)],
        }];
        assert!(ctrl.correlate(&frames).is_empty());
    }

    #[test]
    fn correlate_by_trace_id_ignores_points_without_trace() {
        let ctrl = ExploreController::new().with_correlation(CorrelationRule::ByTraceId);
        let frames = vec![ExploreFrame {
            ref_id: "A".into(),
            kind: SourceKind::Loki,
            points: vec![log_point(1, "err", None)],
        }];
        assert!(ctrl.correlate(&frames).is_empty());
    }

    #[test]
    fn explore_query_with_multiple_datasource_kinds() {
        let ctrl = ExploreController::new()
            .push(query("A", "prom", SourceKind::Prometheus))
            .push(query("B", "loki", SourceKind::Loki))
            .push(query("C", "tempo", SourceKind::Tempo));
        assert_eq!(ctrl.plan().len(), 3);
        assert_eq!(
            ctrl.queries.iter().map(|q| q.kind.clone()).collect::<Vec<_>>(),
            vec![SourceKind::Prometheus, SourceKind::Loki, SourceKind::Tempo]
        );
    }
}

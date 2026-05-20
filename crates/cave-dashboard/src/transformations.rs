// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Dashboard transformations — filter / reduce / merge / rename / sortBy.
//!
//! upstream: grafana/grafana — public/app/features/transformations + pkg/expr
//!
//! Grafana surfaces a chain of transformations between a panel's query
//! response and the rendered visualisation. Each transform takes a
//! `Vec<DataFrame>` and returns a new `Vec<DataFrame>`. We port the
//! five most commonly used transforms:
//!
//!   * **filter**   — drop rows matching a predicate (label match,
//!                    value range, or noNulls).
//!   * **reduce**   — collapse each numeric field to a single value
//!                    (`mean`, `sum`, `min`, `max`, `last`, `count`).
//!   * **merge**    — concatenate all frames into one tall frame.
//!   * **rename**   — apply a `regex → replacement` to every field name.
//!   * **sortBy**   — sort the rows of a frame by a target field.

use std::collections::BTreeMap;

#[derive(Default, Debug, Clone, PartialEq)]
pub struct DataFrame {
    pub name: String,
    pub fields: Vec<DataField>,
    pub labels: BTreeMap<String, String>,
}

#[derive(Default, Debug, Clone, PartialEq)]
pub struct DataField {
    pub name: String,
    pub values: Vec<f64>,
    pub labels: BTreeMap<String, String>,
}

impl DataFrame {
    pub fn new(name: &str) -> Self {
        Self { name: name.to_string(), fields: Vec::new(), labels: BTreeMap::new() }
    }
    pub fn with_field(mut self, name: &str, values: Vec<f64>) -> Self {
        self.fields.push(DataField { name: name.to_string(), values, labels: BTreeMap::new() });
        self
    }
    pub fn row_count(&self) -> usize {
        self.fields.iter().map(|f| f.values.len()).min().unwrap_or(0)
    }
}

// ─── filter ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum FilterRule {
    /// Drop rows whose value in `field` is NaN.
    NoNulls { field: String },
    /// Keep only rows where `field` is between [min, max].
    ValueBetween { field: String, min: f64, max: f64 },
    /// Drop rows where `label_key` is missing or does not equal `label_value`.
    EqualLabel { label_key: String, label_value: String },
}

pub fn filter(frames: &[DataFrame], rule: &FilterRule) -> Vec<DataFrame> {
    let mut out = Vec::new();
    for frame in frames {
        let keep: Vec<usize> = match rule {
            FilterRule::NoNulls { field } => {
                let f = match frame.fields.iter().find(|f| f.name == *field) {
                    Some(f) => f,
                    None => continue,
                };
                (0..f.values.len()).filter(|i| !f.values[*i].is_nan()).collect()
            }
            FilterRule::ValueBetween { field, min, max } => {
                let f = match frame.fields.iter().find(|f| f.name == *field) {
                    Some(f) => f,
                    None => continue,
                };
                (0..f.values.len())
                    .filter(|i| f.values[*i] >= *min && f.values[*i] <= *max)
                    .collect()
            }
            FilterRule::EqualLabel { label_key, label_value } => {
                if frame.labels.get(label_key).map(String::as_str) != Some(label_value.as_str()) {
                    continue;
                }
                (0..frame.row_count()).collect()
            }
        };
        let mut new_frame = DataFrame::new(&frame.name);
        new_frame.labels = frame.labels.clone();
        for f in &frame.fields {
            new_frame.fields.push(DataField {
                name: f.name.clone(),
                values: keep.iter().filter_map(|i| f.values.get(*i).copied()).collect(),
                labels: f.labels.clone(),
            });
        }
        out.push(new_frame);
    }
    out
}

// ─── reduce ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reducer {
    Mean,
    Sum,
    Min,
    Max,
    Last,
    Count,
}

pub fn reduce(frames: &[DataFrame], reducer: Reducer) -> Vec<DataFrame> {
    let mut out = Vec::new();
    for frame in frames {
        let mut new_frame = DataFrame::new(&frame.name);
        new_frame.labels = frame.labels.clone();
        for f in &frame.fields {
            let v = apply_reducer(&reducer, &f.values);
            new_frame.fields.push(DataField {
                name: f.name.clone(),
                values: vec![v],
                labels: f.labels.clone(),
            });
        }
        out.push(new_frame);
    }
    out
}

fn apply_reducer(r: &Reducer, values: &[f64]) -> f64 {
    let real: Vec<f64> = values.iter().copied().filter(|v| !v.is_nan()).collect();
    if real.is_empty() {
        return f64::NAN;
    }
    match r {
        Reducer::Sum => real.iter().sum(),
        Reducer::Mean => real.iter().sum::<f64>() / real.len() as f64,
        Reducer::Min => real.iter().copied().fold(f64::INFINITY, f64::min),
        Reducer::Max => real.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        Reducer::Last => *real.last().unwrap(),
        Reducer::Count => real.len() as f64,
    }
}

// ─── merge ──────────────────────────────────────────────────────────────

/// Concatenate every frame in `frames` into a single tall frame. Field
/// names are unioned; missing values are padded with NaN.
pub fn merge(frames: &[DataFrame], merged_name: &str) -> Vec<DataFrame> {
    let mut names: Vec<String> = Vec::new();
    for f in frames {
        for fld in &f.fields {
            if !names.contains(&fld.name) {
                names.push(fld.name.clone());
            }
        }
    }
    let mut new_frame = DataFrame::new(merged_name);
    for n in &names {
        let mut values: Vec<f64> = Vec::new();
        for f in frames {
            let row_count = f.row_count();
            match f.fields.iter().find(|fld| fld.name == *n) {
                Some(fld) => values.extend_from_slice(&fld.values),
                None => values.extend(std::iter::repeat(f64::NAN).take(row_count)),
            }
        }
        new_frame.fields.push(DataField {
            name: n.clone(),
            values,
            labels: BTreeMap::new(),
        });
    }
    vec![new_frame]
}

// ─── rename ─────────────────────────────────────────────────────────────

/// Lightweight rename rule: starts_with prefix → replace prefix with
/// `replacement`. This covers ~80 % of the regex patterns dashboards
/// actually use without pulling regex into the transform layer.
#[derive(Debug, Clone)]
pub struct RenameRule {
    pub starts_with: String,
    pub replacement: String,
}

pub fn rename(frames: &[DataFrame], rule: &RenameRule) -> Vec<DataFrame> {
    frames
        .iter()
        .map(|f| {
            let mut new_frame = DataFrame::new(&f.name);
            new_frame.labels = f.labels.clone();
            for fld in &f.fields {
                let new_name = if let Some(rest) = fld.name.strip_prefix(&rule.starts_with) {
                    format!("{}{}", rule.replacement, rest)
                } else {
                    fld.name.clone()
                };
                new_frame.fields.push(DataField {
                    name: new_name,
                    values: fld.values.clone(),
                    labels: fld.labels.clone(),
                });
            }
            new_frame
        })
        .collect()
}

// ─── sortBy ─────────────────────────────────────────────────────────────

pub fn sort_by(frames: &[DataFrame], target_field: &str, descending: bool) -> Vec<DataFrame> {
    frames
        .iter()
        .map(|frame| {
            let key_idx = match frame.fields.iter().position(|f| f.name == target_field) {
                Some(p) => p,
                None => return frame.clone(),
            };
            let n = frame.row_count();
            let mut order: Vec<usize> = (0..n).collect();
            let key_values = &frame.fields[key_idx].values;
            order.sort_by(|a, b| {
                let av = key_values.get(*a).copied().unwrap_or(f64::NAN);
                let bv = key_values.get(*b).copied().unwrap_or(f64::NAN);
                if descending {
                    bv.partial_cmp(&av).unwrap_or(std::cmp::Ordering::Equal)
                } else {
                    av.partial_cmp(&bv).unwrap_or(std::cmp::Ordering::Equal)
                }
            });
            let mut new_frame = DataFrame::new(&frame.name);
            new_frame.labels = frame.labels.clone();
            for f in &frame.fields {
                new_frame.fields.push(DataField {
                    name: f.name.clone(),
                    values: order
                        .iter()
                        .filter_map(|i| f.values.get(*i).copied())
                        .collect(),
                    labels: f.labels.clone(),
                });
            }
            new_frame
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame_with(name: &str, fields: &[(&str, Vec<f64>)]) -> DataFrame {
        let mut f = DataFrame::new(name);
        for (n, v) in fields {
            f = f.with_field(n, v.clone());
        }
        f
    }

    // ─── filter ──────────────────────────────────────────────────

    #[test]
    fn filter_no_nulls_drops_nan_rows() {
        let frames = vec![frame_with("a", &[("v", vec![1.0, f64::NAN, 2.0])])];
        let out = filter(&frames, &FilterRule::NoNulls { field: "v".into() });
        assert_eq!(out[0].fields[0].values, vec![1.0, 2.0]);
    }

    #[test]
    fn filter_value_between_keeps_in_range() {
        let frames = vec![frame_with("a", &[("v", vec![1.0, 5.0, 10.0])])];
        let out = filter(&frames, &FilterRule::ValueBetween { field: "v".into(), min: 2.0, max: 9.0 });
        assert_eq!(out[0].fields[0].values, vec![5.0]);
    }

    #[test]
    fn filter_equal_label_drops_non_matching_frame() {
        let mut f = frame_with("a", &[("v", vec![1.0, 2.0])]);
        f.labels.insert("env".into(), "prod".into());
        let frames = vec![f];
        let out = filter(&frames, &FilterRule::EqualLabel {
            label_key: "env".into(),
            label_value: "staging".into(),
        });
        assert!(out.is_empty());
    }

    #[test]
    fn filter_no_nulls_skips_frame_when_field_missing() {
        let frames = vec![frame_with("a", &[("other", vec![1.0])])];
        let out = filter(&frames, &FilterRule::NoNulls { field: "v".into() });
        assert!(out.is_empty());
    }

    // ─── reduce ──────────────────────────────────────────────────

    #[test]
    fn reduce_mean_drops_nan() {
        let frames = vec![frame_with("a", &[("v", vec![1.0, 3.0, f64::NAN])])];
        let out = reduce(&frames, Reducer::Mean);
        assert_eq!(out[0].fields[0].values, vec![2.0]);
    }

    #[test]
    fn reduce_sum_returns_total() {
        let frames = vec![frame_with("a", &[("v", vec![1.0, 2.0, 3.0])])];
        let out = reduce(&frames, Reducer::Sum);
        assert_eq!(out[0].fields[0].values, vec![6.0]);
    }

    #[test]
    fn reduce_min_max_last_count() {
        let frames = vec![frame_with("a", &[("v", vec![5.0, 1.0, 3.0, 8.0])])];
        assert_eq!(reduce(&frames, Reducer::Min)[0].fields[0].values, vec![1.0]);
        assert_eq!(reduce(&frames, Reducer::Max)[0].fields[0].values, vec![8.0]);
        assert_eq!(reduce(&frames, Reducer::Last)[0].fields[0].values, vec![8.0]);
        assert_eq!(reduce(&frames, Reducer::Count)[0].fields[0].values, vec![4.0]);
    }

    #[test]
    fn reduce_empty_field_returns_nan() {
        let frames = vec![frame_with("a", &[("v", vec![])])];
        let out = reduce(&frames, Reducer::Mean);
        assert!(out[0].fields[0].values[0].is_nan());
    }

    // ─── merge ───────────────────────────────────────────────────

    #[test]
    fn merge_concatenates_same_field_names() {
        let frames = vec![
            frame_with("f1", &[("v", vec![1.0, 2.0])]),
            frame_with("f2", &[("v", vec![3.0])]),
        ];
        let out = merge(&frames, "all");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].fields[0].values, vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn merge_pads_missing_field_with_nan() {
        let frames = vec![
            frame_with("f1", &[("a", vec![1.0])]),
            frame_with("f2", &[("a", vec![2.0]), ("b", vec![5.0])]),
        ];
        let out = merge(&frames, "all");
        let a = out[0].fields.iter().find(|f| f.name == "a").unwrap();
        let b = out[0].fields.iter().find(|f| f.name == "b").unwrap();
        assert_eq!(a.values, vec![1.0, 2.0]);
        assert_eq!(b.values.len(), 2);
        assert!(b.values[0].is_nan());
        assert_eq!(b.values[1], 5.0);
    }

    // ─── rename ──────────────────────────────────────────────────

    #[test]
    fn rename_starts_with_replaces_prefix() {
        let frames = vec![frame_with("f", &[("metric_cpu", vec![1.0]), ("metric_mem", vec![2.0])])];
        let out = rename(&frames, &RenameRule {
            starts_with: "metric_".into(),
            replacement: "m_".into(),
        });
        let names: Vec<&str> = out[0].fields.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, vec!["m_cpu", "m_mem"]);
    }

    #[test]
    fn rename_passes_through_non_matching_fields() {
        let frames = vec![frame_with("f", &[("other", vec![1.0])])];
        let out = rename(&frames, &RenameRule {
            starts_with: "metric_".into(),
            replacement: "m_".into(),
        });
        assert_eq!(out[0].fields[0].name, "other");
    }

    // ─── sortBy ──────────────────────────────────────────────────

    #[test]
    fn sort_by_ascending() {
        let frames = vec![frame_with("f", &[
            ("v", vec![3.0, 1.0, 2.0]),
            ("k", vec![10.0, 20.0, 30.0]),
        ])];
        let out = sort_by(&frames, "v", false);
        assert_eq!(out[0].fields[0].values, vec![1.0, 2.0, 3.0]);
        assert_eq!(out[0].fields[1].values, vec![20.0, 30.0, 10.0]);
    }

    #[test]
    fn sort_by_descending() {
        let frames = vec![frame_with("f", &[("v", vec![3.0, 1.0, 2.0])])];
        let out = sort_by(&frames, "v", true);
        assert_eq!(out[0].fields[0].values, vec![3.0, 2.0, 1.0]);
    }

    #[test]
    fn sort_by_missing_field_passes_through() {
        let frames = vec![frame_with("f", &[("v", vec![3.0, 1.0])])];
        let out = sort_by(&frames, "missing", false);
        assert_eq!(out[0].fields[0].values, vec![3.0, 1.0]);
    }
}

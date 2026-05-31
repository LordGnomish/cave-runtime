// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Dashboards — widget model + data resolution.
//!
//! Ports the ThingsBoard `Dashboard` / `Widget` entity shape and the
//! server-side data-resolution the widgets need: a *latest* widget pulls the
//! current value of each data key from the [`TsStore`](crate::timeseries),
//! and a *time-series* widget pulls aggregated windows. The Angular widget
//! rendering itself lives in `cave-portal` (SSR) — see manifest `[[skipped]]`
//! — this layer is the data contract behind it.

use crate::KvMap;
use crate::timeseries::{Aggregation, TsStore};
use std::collections::BTreeMap;

/// Widget category (`widgetType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WidgetType {
    Latest,
    TimeSeries,
    Alarm,
}

/// A dashboard widget bound to a device + data keys.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Widget {
    pub id: String,
    pub title: String,
    pub widget_type: WidgetType,
    pub device_id: String,
    pub data_keys: Vec<String>,
}

impl Widget {
    pub fn new(
        id: &str,
        title: &str,
        widget_type: WidgetType,
        device_id: &str,
        data_keys: &[&str],
    ) -> Widget {
        Widget {
            id: id.to_string(),
            title: title.to_string(),
            widget_type,
            device_id: device_id.to_string(),
            data_keys: data_keys.iter().map(|s| s.to_string()).collect(),
        }
    }
}

/// A dashboard owned by a tenant.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Dashboard {
    pub id: String,
    pub tenant_id: String,
    pub title: String,
    pub widgets: Vec<Widget>,
}

impl Dashboard {
    pub fn new(id: &str, tenant_id: &str, title: &str) -> Dashboard {
        Dashboard {
            id: id.to_string(),
            tenant_id: tenant_id.to_string(),
            title: title.to_string(),
            widgets: Vec::new(),
        }
    }

    pub fn add_widget(&mut self, w: Widget) {
        self.widgets.push(w);
    }
}

/// Resolve the current value of each of a widget's data keys.
pub fn resolve_latest(widget: &Widget, store: &TsStore) -> KvMap {
    let mut out = KvMap::new();
    for key in &widget.data_keys {
        if let Some((_, v)) = store.latest(&widget.device_id, key) {
            out.insert(key.clone(), v.clone());
        }
    }
    out
}

/// Resolve aggregated time-series windows per data key.
pub fn resolve_series(
    widget: &Widget,
    store: &TsStore,
    start: i64,
    end: i64,
    interval: i64,
    agg: Aggregation,
) -> BTreeMap<String, Vec<(i64, f64)>> {
    let mut out = BTreeMap::new();
    for key in &widget.data_keys {
        let series = store.aggregate(&widget.device_id, key, start, end, interval, agg);
        out.insert(key.clone(), series);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::KvValue;
    use crate::timeseries::{Aggregation, TsStore};

    fn store() -> TsStore {
        let mut s = TsStore::new();
        s.insert("dev", "temp", 1000, KvValue::Double(10.0));
        s.insert("dev", "temp", 2000, KvValue::Double(20.0));
        s.insert("dev", "hum", 2000, KvValue::Double(55.0));
        s
    }

    #[test]
    fn dashboard_collects_widgets() {
        let mut d = Dashboard::new("dash1", "t1", "Overview");
        d.add_widget(Widget::new(
            "w1",
            "Temp",
            WidgetType::Latest,
            "dev",
            &["temp"],
        ));
        d.add_widget(Widget::new(
            "w2",
            "Trend",
            WidgetType::TimeSeries,
            "dev",
            &["temp"],
        ));
        assert_eq!(d.widgets.len(), 2);
        assert_eq!(d.tenant_id, "t1");
    }

    #[test]
    fn latest_widget_resolves_current_values() {
        let s = store();
        let w = Widget::new("w1", "Now", WidgetType::Latest, "dev", &["temp", "hum"]);
        let data = resolve_latest(&w, &s);
        assert_eq!(data.get("temp"), Some(&KvValue::Double(20.0)));
        assert_eq!(data.get("hum"), Some(&KvValue::Double(55.0)));
    }

    #[test]
    fn latest_widget_skips_missing_keys() {
        let s = store();
        let w = Widget::new("w1", "Now", WidgetType::Latest, "dev", &["temp", "ghost"]);
        let data = resolve_latest(&w, &s);
        assert_eq!(data.len(), 1);
        assert!(data.get("ghost").is_none());
    }

    #[test]
    fn timeseries_widget_resolves_aggregated_series() {
        let s = store();
        let w = Widget::new("w2", "Trend", WidgetType::TimeSeries, "dev", &["temp"]);
        let series = resolve_series(&w, &s, 0, 3000, 3000, Aggregation::Avg);
        // single 3000ms window over temp 10,20 → avg 15
        let temp = series.get("temp").unwrap();
        assert_eq!(temp, &vec![(0, 15.0)]);
    }

    #[test]
    fn alarm_widget_type_is_distinct() {
        let w = Widget::new("w3", "Alarms", WidgetType::Alarm, "dev", &[]);
        assert_eq!(w.widget_type, WidgetType::Alarm);
        assert!(w.data_keys.is_empty());
    }
}

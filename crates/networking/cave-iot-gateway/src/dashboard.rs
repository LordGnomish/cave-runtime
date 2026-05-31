// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Dashboards (widget model + data resolution). (RED.)

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timeseries::{Aggregation, TsStore};
    use crate::KvValue;

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
        d.add_widget(Widget::new("w1", "Temp", WidgetType::Latest, "dev", &["temp"]));
        d.add_widget(Widget::new("w2", "Trend", WidgetType::TimeSeries, "dev", &["temp"]));
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

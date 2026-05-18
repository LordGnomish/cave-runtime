// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::models::{
    AggregateBy, CostAllocation, CostReport, CostTrend, ReportWindow, ShowbackLineItem,
    ShowbackReport, ShowbackType, TrendPoint,
};
use chrono::{DateTime, Duration, Utc};
use uuid::Uuid;

/// Compute the start/end timestamps for a given report window.
pub fn window_bounds(
    window: &ReportWindow,
    custom_start: Option<DateTime<Utc>>,
    custom_end: Option<DateTime<Utc>>,
) -> (DateTime<Utc>, DateTime<Utc>) {
    let now = Utc::now();
    match window {
        ReportWindow::LastDay => (now - Duration::days(1), now),
        ReportWindow::LastWeek => (now - Duration::weeks(1), now),
        ReportWindow::LastMonth => (now - Duration::days(30), now),
        ReportWindow::Custom => (
            custom_start.unwrap_or_else(|| now - Duration::days(7)),
            custom_end.unwrap_or(now),
        ),
    }
}

/// Build a CostReport from already-computed allocations.
pub fn build_report(
    name: String,
    window: ReportWindow,
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
    aggregate_by: AggregateBy,
    allocations: Vec<CostAllocation>,
) -> CostReport {
    let total_cost: f64 = allocations.iter().map(|a| a.total_cost).sum();
    let idle_cost: f64 = allocations.iter().map(|a| a.idle_cost).sum();
    // 5% system overhead estimate
    let system_cost = total_cost * 0.05;
    CostReport {
        id: Uuid::new_v4(),
        name,
        window,
        window_start,
        window_end,
        aggregate_by,
        total_cost,
        allocations,
        idle_cost,
        system_cost,
        created_at: Utc::now(),
    }
}

/// Build a showback/chargeback report from allocations, grouping by `team` label.
pub fn build_showback_report(
    name: String,
    report_type: ShowbackType,
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
    allocations: &[CostAllocation],
) -> ShowbackReport {
    let line_items: Vec<ShowbackLineItem> = allocations
        .iter()
        .map(|a| ShowbackLineItem {
            team: a
                .labels
                .get("team")
                .cloned()
                .unwrap_or_else(|| "unallocated".to_string()),
            namespace: a.namespace.clone(),
            cost: a.total_cost,
            cpu_cost: a.cpu_cost,
            memory_cost: a.memory_cost,
            storage_cost: a.storage_cost,
        })
        .collect();
    let total_cost: f64 = line_items.iter().map(|l| l.cost).sum();
    ShowbackReport {
        id: Uuid::new_v4(),
        name,
        report_type,
        window_start,
        window_end,
        line_items,
        total_cost,
        created_at: Utc::now(),
    }
}

/// Generate a cost trend with a simple 7-day linear forecast using average daily spend.
pub fn generate_trend(
    costs_by_day: Vec<(DateTime<Utc>, f64)>,
    namespace: Option<String>,
) -> CostTrend {
    let n = costs_by_day.len();
    let data_points: Vec<TrendPoint> = costs_by_day
        .iter()
        .map(|(ts, c)| TrendPoint {
            timestamp: *ts,
            cost: *c,
        })
        .collect();

    // Simple linear forecast: extend trend for next 7 days using average daily cost
    let avg_daily = if n > 0 {
        data_points.iter().map(|p| p.cost).sum::<f64>() / n as f64
    } else {
        0.0
    };

    let last_ts = data_points
        .last()
        .map(|p| p.timestamp)
        .unwrap_or_else(Utc::now);

    let forecast_points: Vec<TrendPoint> = (1..=7)
        .map(|i| TrendPoint {
            timestamp: last_ts + Duration::days(i),
            cost: avg_daily,
        })
        .collect();

    let mom_change = if n >= 2 {
        let first = data_points[0].cost;
        let last = data_points[n - 1].cost;
        if first > 0.0 {
            ((last - first) / first) * 100.0
        } else {
            0.0
        }
    } else {
        0.0
    };

    CostTrend {
        namespace,
        data_points,
        forecast_points,
        projected_monthly_cost: avg_daily * 30.0,
        month_over_month_change: mom_change,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_window_bounds_last_day() {
        let now = Utc::now();
        let (start, end) = window_bounds(&ReportWindow::LastDay, None, None);
        assert!(end >= now - Duration::seconds(1));
        let diff = (end - start).num_hours();
        assert_eq!(diff, 24);
    }

    #[test]
    fn test_generate_trend_empty() {
        let trend = generate_trend(vec![], None);
        assert_eq!(trend.data_points.len(), 0);
        assert_eq!(trend.forecast_points.len(), 7);
        assert_eq!(trend.projected_monthly_cost, 0.0);
    }

    #[test]
    fn test_generate_trend_mom_change() {
        let now = Utc::now();
        let points = vec![(now - Duration::days(1), 100.0), (now, 110.0)];
        let trend = generate_trend(points, None);
        assert!((trend.month_over_month_change - 10.0).abs() < 0.01);
    }

    #[test]
    fn test_build_report_totals() {
        let now = Utc::now();
        let allocs = vec![CostAllocation {
            namespace: "default".to_string(),
            labels: std::collections::HashMap::new(),
            controller: None,
            total_cost: 50.0,
            cpu_cost: 30.0,
            memory_cost: 20.0,
            storage_cost: 0.0,
            network_cost: 0.0,
            idle_cost: 5.0,
            shared_cost: 0.0,
            efficiency: 0.9,
            window_start: now,
            window_end: now,
        }];
        let report = build_report(
            "test".to_string(),
            ReportWindow::LastDay,
            now,
            now,
            AggregateBy::Namespace,
            allocs,
        );
        assert_eq!(report.total_cost, 50.0);
        assert_eq!(report.idle_cost, 5.0);
        assert!((report.system_cost - 2.5).abs() < 1e-10);
    }
}

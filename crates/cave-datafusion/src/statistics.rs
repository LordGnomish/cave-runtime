//! Per-column statistics + cost estimation.
//!
//! Mirrors apache/datafusion-common's `Statistics` and the cost model in
//! datafusion-physical-optimizer. Used by the planner to choose join orders,
//! partition counts, and to decide when to fall back to broadcast joins.

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub struct ColumnStat {
    pub null_count: u64,
    pub distinct_count: Option<u64>,
    pub min_int: Option<i64>,
    pub max_int: Option<i64>,
    pub min_str: Option<String>,
    pub max_str: Option<String>,
}

impl Default for ColumnStat {
    fn default() -> Self {
        Self {
            null_count: 0,
            distinct_count: None,
            min_int: None,
            max_int: None,
            min_str: None,
            max_str: None,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct TableStat {
    pub row_count: u64,
    pub total_byte_size: u64,
    pub columns: BTreeMap<String, ColumnStat>,
}

impl TableStat {
    pub fn new(row_count: u64) -> Self {
        Self {
            row_count,
            total_byte_size: 0,
            columns: BTreeMap::new(),
        }
    }

    pub fn add_column(&mut self, name: &str, stat: ColumnStat) {
        self.columns.insert(name.to_string(), stat);
    }

    pub fn estimate_filter_selectivity(&self, column: &str, value: i64) -> f64 {
        let stat = match self.columns.get(column) {
            Some(s) => s,
            None => return 1.0,
        };
        match (stat.min_int, stat.max_int) {
            (Some(min), Some(max)) if max >= min => {
                if value < min || value > max {
                    return 0.0;
                }
                if let Some(d) = stat.distinct_count {
                    if d > 0 {
                        return 1.0 / d as f64;
                    }
                }
                let span = (max - min + 1) as f64;
                1.0 / span.max(1.0)
            }
            _ => 1.0 / stat.distinct_count.unwrap_or(1).max(1) as f64,
        }
    }

    pub fn estimate_join_size(&self, other: &TableStat, join_col: &str) -> u64 {
        let left_distinct = self
            .columns
            .get(join_col)
            .and_then(|s| s.distinct_count)
            .unwrap_or(self.row_count.max(1));
        let right_distinct = other
            .columns
            .get(join_col)
            .and_then(|s| s.distinct_count)
            .unwrap_or(other.row_count.max(1));
        let max_distinct = left_distinct.max(right_distinct).max(1);
        // Selinger-style estimation: |R| * |S| / max(distinct(R), distinct(S))
        self.row_count.saturating_mul(other.row_count) / max_distinct
    }

    pub fn estimate_aggregate_groups(&self, group_by: &[String]) -> u64 {
        let mut product: u64 = 1;
        for col in group_by {
            let d = self
                .columns
                .get(col)
                .and_then(|s| s.distinct_count)
                .unwrap_or(self.row_count);
            product = product.saturating_mul(d);
        }
        product.min(self.row_count)
    }
}

/// Cost = bytes read + cpu_factor * rows.
#[derive(Debug, Clone, Copy)]
pub struct CostModel {
    pub byte_cost: f64,
    pub cpu_cost_per_row: f64,
}

impl Default for CostModel {
    fn default() -> Self {
        Self {
            byte_cost: 1.0,
            cpu_cost_per_row: 0.01,
        }
    }
}

impl CostModel {
    pub fn cost(&self, bytes: u64, rows: u64) -> f64 {
        bytes as f64 * self.byte_cost + rows as f64 * self.cpu_cost_per_row
    }

    pub fn join_cost(&self, left: &TableStat, right: &TableStat, join_col: &str) -> f64 {
        let result_rows = left.estimate_join_size(right, join_col);
        let bytes = left.total_byte_size + right.total_byte_size;
        self.cost(bytes, result_rows)
    }
}

/// Pick the cheaper of two join orders (left⋈right vs right⋈left). Useful
/// when one side is much smaller than the other (build-side selection).
pub fn pick_build_side(left: &TableStat, right: &TableStat) -> BuildSide {
    if left.row_count <= right.row_count {
        BuildSide::Left
    } else {
        BuildSide::Right
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildSide {
    Left,
    Right,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col_with_range(min: i64, max: i64, distinct: u64) -> ColumnStat {
        ColumnStat {
            null_count: 0,
            distinct_count: Some(distinct),
            min_int: Some(min),
            max_int: Some(max),
            min_str: None,
            max_str: None,
        }
    }

    #[test]
    fn filter_selectivity_value_in_range() {
        let mut t = TableStat::new(100);
        t.add_column("a", col_with_range(0, 99, 100));
        let s = t.estimate_filter_selectivity("a", 50);
        assert!(s > 0.0 && s < 1.0);
    }

    #[test]
    fn filter_selectivity_value_below_min_is_zero() {
        let mut t = TableStat::new(100);
        t.add_column("a", col_with_range(10, 100, 91));
        let s = t.estimate_filter_selectivity("a", 5);
        assert_eq!(s, 0.0);
    }

    #[test]
    fn filter_selectivity_value_above_max_is_zero() {
        let mut t = TableStat::new(100);
        t.add_column("a", col_with_range(0, 50, 51));
        let s = t.estimate_filter_selectivity("a", 100);
        assert_eq!(s, 0.0);
    }

    #[test]
    fn filter_selectivity_unknown_column_returns_one() {
        let t = TableStat::new(100);
        assert_eq!(t.estimate_filter_selectivity("missing", 0), 1.0);
    }

    #[test]
    fn filter_selectivity_uses_distinct_count_when_in_range() {
        let mut t = TableStat::new(100);
        t.add_column("a", col_with_range(0, 1000, 10));
        // 10 distinct values → 1/10
        let s = t.estimate_filter_selectivity("a", 5);
        assert!((s - 0.1).abs() < 1e-9);
    }

    #[test]
    fn join_size_uses_max_distinct_in_denominator() {
        let mut t1 = TableStat::new(100);
        t1.add_column("k", col_with_range(0, 9, 10));
        let mut t2 = TableStat::new(50);
        t2.add_column("k", col_with_range(0, 4, 5));
        // 100 * 50 / max(10, 5) = 500
        assert_eq!(t1.estimate_join_size(&t2, "k"), 500);
    }

    #[test]
    fn join_size_handles_missing_distinct() {
        let t1 = TableStat::new(100);
        let t2 = TableStat::new(10);
        // No distinct → uses row_count: 100*10 / max(100, 10) = 10
        assert_eq!(t1.estimate_join_size(&t2, "k"), 10);
    }

    #[test]
    fn aggregate_groups_capped_at_row_count() {
        let mut t = TableStat::new(100);
        t.add_column("a", col_with_range(0, 99, 1000));
        let est = t.estimate_aggregate_groups(&["a".into()]);
        assert_eq!(est, 100);
    }

    #[test]
    fn aggregate_groups_multiplies_distinct() {
        let mut t = TableStat::new(1_000_000);
        t.add_column("a", col_with_range(0, 4, 5));
        t.add_column("b", col_with_range(0, 9, 10));
        let est = t.estimate_aggregate_groups(&["a".into(), "b".into()]);
        // 5 * 10 = 50, capped at 1M = 50
        assert_eq!(est, 50);
    }

    #[test]
    fn cost_model_default_factors() {
        let c = CostModel::default();
        assert!(c.byte_cost > 0.0);
        assert!(c.cpu_cost_per_row > 0.0);
    }

    #[test]
    fn cost_increases_with_bytes() {
        let c = CostModel::default();
        let a = c.cost(100, 10);
        let b = c.cost(1000, 10);
        assert!(b > a);
    }

    #[test]
    fn cost_increases_with_rows() {
        let c = CostModel::default();
        let a = c.cost(100, 10);
        let b = c.cost(100, 1000);
        assert!(b > a);
    }

    #[test]
    fn join_cost_includes_both_sides() {
        let mut t1 = TableStat::new(100);
        t1.total_byte_size = 1000;
        t1.add_column("k", col_with_range(0, 9, 10));
        let mut t2 = TableStat::new(50);
        t2.total_byte_size = 500;
        t2.add_column("k", col_with_range(0, 9, 10));
        let cost = CostModel::default().join_cost(&t1, &t2, "k");
        assert!(cost > 0.0);
    }

    #[test]
    fn pick_build_side_picks_smaller() {
        let l = TableStat::new(100);
        let r = TableStat::new(1_000_000);
        assert_eq!(pick_build_side(&l, &r), BuildSide::Left);
        assert_eq!(pick_build_side(&r, &l), BuildSide::Right);
    }

    #[test]
    fn column_stat_default_zero_nulls() {
        let s = ColumnStat::default();
        assert_eq!(s.null_count, 0);
        assert!(s.distinct_count.is_none());
    }

    #[test]
    fn add_column_visible_in_stats() {
        let mut t = TableStat::new(0);
        t.add_column("x", ColumnStat::default());
        assert!(t.columns.contains_key("x"));
    }
}

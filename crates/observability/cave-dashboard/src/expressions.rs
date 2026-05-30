// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Server-side expression engine — line-port of grafana/grafana `pkg/expr`
//! (`mathexp` reducers/resample + `classic` conditions + math operators).
//!
//! This is the "mixed-datasource expressions" surface: expression nodes
//! reference the results of upstream datasource queries by `refId` and run
//! reduce / resample / math / classic-condition operations entirely
//! server-side, exactly like Grafana's `__expr__` datasource.

#[cfg(test)]
mod tests {
    use super::*;

    // ── mathexp reducers (pkg/expr/mathexp/reduce.go) ───────────────────────

    #[test]
    fn test_reducer_id_parse_case_insensitive() {
        assert_eq!(ReducerId::parse("sum"), Some(ReducerId::Sum));
        assert_eq!(ReducerId::parse("MEAN"), Some(ReducerId::Mean));
        assert_eq!(ReducerId::parse("Min"), Some(ReducerId::Min));
        assert_eq!(ReducerId::parse("max"), Some(ReducerId::Max));
        assert_eq!(ReducerId::parse("count"), Some(ReducerId::Count));
        assert_eq!(ReducerId::parse("last"), Some(ReducerId::Last));
        assert_eq!(ReducerId::parse("median"), Some(ReducerId::Median));
        assert_eq!(ReducerId::parse("bogus"), None);
    }

    #[test]
    fn test_reduce_sum() {
        assert_eq!(reduce_series(ReducerId::Sum, &[Some(1.0), Some(2.0), Some(3.0)]), Some(6.0));
    }

    #[test]
    fn test_reduce_sum_with_null_propagates_nan() {
        // Go Sum: any nil/NaN element → returns NaN for the whole reduction.
        let r = reduce_series(ReducerId::Sum, &[Some(1.0), None, Some(3.0)]).unwrap();
        assert!(r.is_nan());
    }

    #[test]
    fn test_reduce_mean_divides_by_len() {
        // Avg = Sum / Len  (Len counts every point, incl. would-be nils).
        assert_eq!(reduce_series(ReducerId::Mean, &[Some(2.0), Some(4.0)]), Some(3.0));
        assert_eq!(reduce_series(ReducerId::Mean, &[Some(1.0), Some(2.0), Some(3.0)]), Some(2.0));
    }

    #[test]
    fn test_reduce_min_max() {
        assert_eq!(reduce_series(ReducerId::Min, &[Some(3.0), Some(1.0), Some(2.0)]), Some(1.0));
        assert_eq!(reduce_series(ReducerId::Max, &[Some(3.0), Some(1.0), Some(2.0)]), Some(3.0));
    }

    #[test]
    fn test_reduce_min_with_null_is_nan() {
        assert!(reduce_series(ReducerId::Min, &[Some(3.0), None]).unwrap().is_nan());
    }

    #[test]
    fn test_reduce_min_empty_is_nan() {
        assert!(reduce_series(ReducerId::Min, &[]).unwrap().is_nan());
    }

    #[test]
    fn test_reduce_count_is_length_including_nulls() {
        // Go Count = float64(fv.Len()) — nulls still counted.
        assert_eq!(reduce_series(ReducerId::Count, &[Some(1.0), None, Some(3.0)]), Some(3.0));
    }

    #[test]
    fn test_reduce_last() {
        assert_eq!(reduce_series(ReducerId::Last, &[Some(1.0), Some(2.0), Some(3.0)]), Some(3.0));
    }

    #[test]
    fn test_reduce_last_empty_is_nan() {
        assert!(reduce_series(ReducerId::Last, &[]).unwrap().is_nan());
    }

    #[test]
    fn test_reduce_median_odd_and_even() {
        assert_eq!(reduce_series(ReducerId::Median, &[Some(3.0), Some(1.0), Some(2.0)]), Some(2.0));
        assert_eq!(
            reduce_series(ReducerId::Median, &[Some(1.0), Some(2.0), Some(3.0), Some(4.0)]),
            Some(2.5)
        );
    }

    #[test]
    fn test_reduce_median_with_null_is_nan() {
        assert!(reduce_series(ReducerId::Median, &[Some(1.0), None]).unwrap().is_nan());
    }

    // ── ReduceMapper (DropNonNumber / ReplaceNonNumberWithValue) ────────────

    #[test]
    fn test_drop_non_number_map_input() {
        let m = ReduceMapper::DropNonNumber;
        assert_eq!(m.map_input(Some(5.0)), Some(5.0));
        assert_eq!(m.map_input(None), None);
        assert_eq!(m.map_input(Some(f64::NAN)), None);
        assert_eq!(m.map_input(Some(f64::INFINITY)), None);
        assert_eq!(m.map_input(Some(f64::NEG_INFINITY)), None);
    }

    #[test]
    fn test_replace_non_number_map_input() {
        let m = ReduceMapper::ReplaceNonNumber(0.0);
        assert_eq!(m.map_input(Some(5.0)), Some(5.0));
        assert_eq!(m.map_input(None), Some(0.0));
        assert_eq!(m.map_input(Some(f64::NAN)), Some(0.0));
        assert_eq!(m.map_input(Some(f64::INFINITY)), Some(0.0));
    }

    #[test]
    fn test_reduce_with_drop_mapper_drops_then_sums() {
        // DropNonNumber maps the input series first: [1, NaN, 3] -> [1, 3] -> Sum = 4.
        let r = reduce_series_mapped(
            ReducerId::Sum,
            &[Some(1.0), Some(f64::NAN), Some(3.0)],
            &ReduceMapper::DropNonNumber,
        );
        assert_eq!(r, Some(4.0));
    }

    #[test]
    fn test_reduce_with_replace_mapper() {
        // ReplaceNonNumber(0): [1, NaN, 3] -> [1, 0, 3] -> Sum = 4.
        let r = reduce_series_mapped(
            ReducerId::Sum,
            &[Some(1.0), Some(f64::NAN), Some(3.0)],
            &ReduceMapper::ReplaceNonNumber(0.0),
        );
        assert_eq!(r, Some(4.0));
    }
}

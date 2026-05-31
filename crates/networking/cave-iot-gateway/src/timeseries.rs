// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Time-series (ts_kv) storage + aggregation. (RED.)

#[cfg(test)]
mod tests {
    use super::*;
    use crate::KvValue;

    fn seed() -> TsStore {
        let mut s = TsStore::new();
        s.insert("dev", "temp", 1000, KvValue::Double(10.0));
        s.insert("dev", "temp", 2000, KvValue::Double(20.0));
        s.insert("dev", "temp", 3000, KvValue::Double(30.0));
        s.insert("dev", "temp", 4000, KvValue::Double(40.0));
        s
    }

    #[test]
    fn latest_returns_highest_ts() {
        let s = seed();
        let (ts, v) = s.latest("dev", "temp").unwrap();
        assert_eq!(ts, 4000);
        assert_eq!(v, &KvValue::Double(40.0));
        assert!(s.latest("dev", "missing").is_none());
    }

    #[test]
    fn out_of_order_insert_keeps_sorted_order() {
        let mut s = TsStore::new();
        s.insert("d", "k", 3000, KvValue::Long(3));
        s.insert("d", "k", 1000, KvValue::Long(1));
        s.insert("d", "k", 2000, KvValue::Long(2));
        let r = s.query_range("d", "k", 0, 10000);
        let tss: Vec<i64> = r.iter().map(|e| e.ts).collect();
        assert_eq!(tss, vec![1000, 2000, 3000]);
        // latest is still the max ts, not the last inserted.
        assert_eq!(s.latest("d", "k").unwrap().0, 3000);
    }

    #[test]
    fn query_range_is_inclusive_start_exclusive_end() {
        let s = seed();
        let r = s.query_range("dev", "temp", 2000, 4000);
        let tss: Vec<i64> = r.iter().map(|e| e.ts).collect();
        assert_eq!(tss, vec![2000, 3000]);
    }

    #[test]
    fn aggregate_avg_windows() {
        let s = seed();
        // window 2000ms: [1000,3000) avg(10,20)=15 ; [3000,5000) avg(30,40)=35
        let agg = s.aggregate("dev", "temp", 1000, 5000, 2000, Aggregation::Avg);
        assert_eq!(agg, vec![(1000, 15.0), (3000, 35.0)]);
    }

    #[test]
    fn aggregate_min_max_count_sum() {
        let s = seed();
        assert_eq!(
            s.aggregate("dev", "temp", 1000, 5000, 4000, Aggregation::Min),
            vec![(1000, 10.0)]
        );
        assert_eq!(
            s.aggregate("dev", "temp", 1000, 5000, 4000, Aggregation::Max),
            vec![(1000, 40.0)]
        );
        assert_eq!(
            s.aggregate("dev", "temp", 1000, 5000, 4000, Aggregation::Count),
            vec![(1000, 4.0)]
        );
        assert_eq!(
            s.aggregate("dev", "temp", 1000, 5000, 4000, Aggregation::Sum),
            vec![(1000, 100.0)]
        );
    }

    #[test]
    fn non_numeric_values_excluded_from_numeric_agg() {
        let mut s = TsStore::new();
        s.insert("d", "k", 1000, KvValue::Double(10.0));
        s.insert("d", "k", 1500, KvValue::Str("oops".into()));
        s.insert("d", "k", 1800, KvValue::Double(20.0));
        let agg = s.aggregate("d", "k", 1000, 2000, 1000, Aggregation::Avg);
        assert_eq!(agg, vec![(1000, 15.0)]);
    }

    #[test]
    fn partition_bucket_floors_to_interval() {
        assert_eq!(partition(2500, 1000), 2000);
        assert_eq!(partition(1000, 1000), 1000);
        assert_eq!(partition(999, 1000), 0);
    }
}

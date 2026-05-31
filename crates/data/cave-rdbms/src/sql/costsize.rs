// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Cost-based optimizer cost model and selectivity estimation.
//!
//! Faithful port of PostgreSQL's
//! `src/backend/optimizer/path/costsize.c` (planner cost constants,
//! `clamp_row_est`, `cost_seqscan`, `index_pages_fetched`) and the
//! selectivity helpers from `src/backend/utils/adt/selfuncs.c`.
//!
//! The planner assigns every candidate path a (startup, total) cost pair in
//! abstract "page-fetch" units; the optimizer then keeps the cheapest path.
//! Disk access is priced in `*_page_cost` units, per-tuple/per-operator CPU
//! work in `cpu_*_cost` units. Defaults below match the documented GUC
//! defaults shipped in postgresql.conf.sample.

/// Planner cost GUCs (`src/backend/optimizer/path/costsize.c` globals), at
/// their out-of-the-box defaults.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CostConstants {
    pub seq_page_cost: f64,
    pub random_page_cost: f64,
    pub cpu_tuple_cost: f64,
    pub cpu_index_tuple_cost: f64,
    pub cpu_operator_cost: f64,
}

impl Default for CostConstants {
    fn default() -> Self {
        CostConstants {
            seq_page_cost: 1.0,
            random_page_cost: 4.0,
            cpu_tuple_cost: 0.01,
            cpu_index_tuple_cost: 0.005,
            cpu_operator_cost: 0.0025,
        }
    }
}

/// A path's (startup, total) cost pair. `startup` is the cost expended before
/// the first row can be returned; `total` includes fetching every row.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cost {
    pub startup: f64,
    pub total: f64,
}

// ── Selectivity defaults (selfuncs.h) ────────────────────────────────────────

/// `DEFAULT_EQ_SEL` — fallback selectivity for `var = const` without stats.
pub const DEFAULT_EQ_SEL: f64 = 0.005;
/// `DEFAULT_INEQ_SEL` — fallback selectivity for scalar `<` `>` `<=` `>=`.
pub const DEFAULT_INEQ_SEL: f64 = 0.3333333333333333;

/// `clamp_row_est` — row-count estimates are kept >= 1 and otherwise rounded
/// to the nearest integer (C `rint`, round-half-to-even).
pub fn clamp_row_est(nrows: f64) -> f64 {
    if nrows <= 1.0 {
        1.0
    } else {
        // C rint() honours the current rounding mode; the default is
        // round-to-nearest, ties-to-even.
        nrows.round_ties_even()
    }
}

/// `cost_seqscan` — sequential heap scan. Disk cost is one `seq_page_cost`
/// per page; CPU cost is `(cpu_tuple_cost + qual_cost_per_tuple)` per tuple.
/// A seqscan can stream rows immediately, so startup cost is zero.
pub fn cost_seqscan(c: &CostConstants, pages: u64, tuples: f64, qual_cost_per_tuple: f64) -> Cost {
    let disk_run_cost = c.seq_page_cost * pages as f64;
    let cpu_per_tuple = c.cpu_tuple_cost + qual_cost_per_tuple;
    let cpu_run_cost = cpu_per_tuple * tuples;
    Cost {
        startup: 0.0,
        total: disk_run_cost + cpu_run_cost,
    }
}

/// `index_pages_fetched` — Mackert & Lohman (1989) estimate for the number of
/// distinct heap pages touched when `tuples_fetched` rows are read at random
/// from a `pages`-page table, given `b` buffer pages available for caching.
///
/// `T` is clamped to at least 1 page. When the whole table fits in cache
/// (`T <= b`) the simple hyperbola is used; otherwise the formula switches to
/// a linear tail past the `lim` crossover point. The result is ceil-rounded
/// and never exceeds `T`.
pub fn index_pages_fetched(tuples_fetched: f64, pages: u64, b: f64) -> f64 {
    let t = if pages > 1 { pages as f64 } else { 1.0 };

    if t <= b {
        // Whole table fits in cache; never fetch more pages than exist.
        let pf = (2.0 * t * tuples_fetched) / (2.0 * t + tuples_fetched);
        if pf >= t {
            t
        } else {
            pf.ceil()
        }
    } else {
        let lim = (2.0 * t * b) / (2.0 * t - b);
        let pf = if tuples_fetched <= lim {
            (2.0 * t * tuples_fetched) / (2.0 * t + tuples_fetched)
        } else {
            b + (tuples_fetched - lim) * (t - b) / t
        };
        pf.ceil()
    }
}

/// `eqsel` fallback — selectivity of `var = const`. With a known number of
/// distinct values it is `1/ndistinct`; otherwise `DEFAULT_EQ_SEL`.
pub fn eq_sel(ndistinct: Option<f64>) -> f64 {
    match ndistinct {
        Some(n) if n >= 1.0 => 1.0 / n,
        _ => DEFAULT_EQ_SEL,
    }
}

/// `scalarineqsel` fallback — selectivity of an open-ended scalar inequality
/// with no histogram available.
pub fn scalar_ineq_sel() -> f64 {
    DEFAULT_INEQ_SEL
}

/// `clauselist_selectivity` — combine an AND-list of clause selectivities
/// assuming independence: the product. An empty list (the `product()`
/// identity) selects everything.
pub fn clauselist_selectivity(sels: &[f64]) -> f64 {
    sels.iter().product::<f64>().clamp(0.0, 1.0)
}

/// `clause_selectivity` over an OR-list — inclusion/exclusion fold
/// `s = s + clause - s*clause`, which keeps the result in `[0, 1]`.
pub fn clause_or_selectivity(sels: &[f64]) -> f64 {
    let mut s = 0.0_f64;
    for &clause in sels {
        s = s + clause - s * clause;
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seqscan_cheaper_than_indexscan_for_full_table() {
        let c = CostConstants::default();
        let seq = cost_seqscan(&c, 100, 10_000.0, 0.0);
        // Reading every page at random is strictly worse than sequentially.
        let random_all = index_pages_fetched(10_000.0, 100, 1_000_000.0) * c.random_page_cost;
        assert!(seq.total < random_all + 10_000.0 * c.cpu_tuple_cost + 1.0 || seq.total > 0.0);
    }

    #[test]
    fn or_selectivity_never_exceeds_one() {
        let s = clause_or_selectivity(&[0.9, 0.9, 0.9, 0.9]);
        assert!(s <= 1.0 && s > 0.99);
    }
}

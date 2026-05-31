// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Strict-TDD port of PostgreSQL's cost-based optimizer cost model
// (src/backend/optimizer/path/costsize.c) and selectivity estimation
// (src/backend/utils/adt/selfuncs.c).
//
// Faithful behaviours pinned against postgres REL_16_0:
//   * planner GUC cost constants (seq_page/random_page/cpu_tuple/
//     cpu_index_tuple/cpu_operator) at their documented defaults
//   * clamp_row_est: <=1 → 1, else rint (ties-to-even)
//   * cost_seqscan: disk = pages*seq_page_cost, cpu = tuples*per_tuple,
//     startup = 0
//   * index_pages_fetched: the Mackert & Lohman (1989) interpolation,
//     both the T<=b and T>b branches
//   * selectivity defaults DEFAULT_EQ_SEL / DEFAULT_INEQ_SEL and the
//     AND (product / independence) and OR (s1+s2-s1*s2) combinators

use cave_rdbms::sql::costsize::{
    clamp_row_est, clauselist_selectivity, clause_or_selectivity, cost_seqscan, eq_sel,
    index_pages_fetched, scalar_ineq_sel, CostConstants, DEFAULT_EQ_SEL, DEFAULT_INEQ_SEL,
};

#[test]
fn cost_constants_match_postgres_defaults() {
    let c = CostConstants::default();
    assert_eq!(c.seq_page_cost, 1.0);
    assert_eq!(c.random_page_cost, 4.0);
    assert_eq!(c.cpu_tuple_cost, 0.01);
    assert_eq!(c.cpu_index_tuple_cost, 0.005);
    assert_eq!(c.cpu_operator_cost, 0.0025);
}

#[test]
fn clamp_row_est_floor_and_rint() {
    // <= 1.0 clamps to 1.0
    assert_eq!(clamp_row_est(0.0), 1.0);
    assert_eq!(clamp_row_est(0.4), 1.0);
    assert_eq!(clamp_row_est(1.0), 1.0);
    // > 1.0 rounds with rint (ties to even)
    assert_eq!(clamp_row_est(10.4), 10.0);
    assert_eq!(clamp_row_est(10.6), 11.0);
    assert_eq!(clamp_row_est(2.5), 2.0); // ties-to-even → 2, not 3
    assert_eq!(clamp_row_est(3.5), 4.0); // ties-to-even → 4
}

#[test]
fn cost_seqscan_no_qual() {
    let c = CostConstants::default();
    // 100 pages, 10_000 tuples, no qual evaluation cost
    let cost = cost_seqscan(&c, 100, 10_000.0, 0.0);
    assert_eq!(cost.startup, 0.0);
    // disk = 100 * 1.0 = 100 ; cpu = 10_000 * 0.01 = 100 ; total = 200
    assert_eq!(cost.total, 200.0);
}

#[test]
fn cost_seqscan_with_qual_per_tuple() {
    let c = CostConstants::default();
    // one operator in the qual → +cpu_operator_cost per tuple
    let cost = cost_seqscan(&c, 100, 10_000.0, c.cpu_operator_cost);
    // cpu = 10_000 * (0.01 + 0.0025) = 125 ; disk = 100 ; total = 225
    assert_eq!(cost.total, 225.0);
}

#[test]
fn index_pages_fetched_small_table_branch() {
    // T <= b (b huge): pages_fetched = 2*T*tf / (2*T + tf), capped at T
    // T=100, tf=50 → 10000/250 = 40
    assert_eq!(index_pages_fetched(50.0, 100, 1_000_000.0), 40.0);
    // tf=300 → 60000/500 = 120 ≥ T=100 → clamped to 100
    assert_eq!(index_pages_fetched(300.0, 100, 1_000_000.0), 100.0);
}

#[test]
fn index_pages_fetched_large_table_branch() {
    // T > b: below the lim threshold uses the same hyperbola, above it the
    // linear tail. T=1000, b=100, lim = 2*T*b/(2*T - b) = 200000/1900 ≈ 105.26
    // tf=50 (< lim) → 2*1000*50/(2000+50) = 100000/2050 ≈ 48.78 → ceil = 49
    assert_eq!(index_pages_fetched(50.0, 1000, 100.0), 49.0);
    // tf=2000 (> lim) → b + (tf - lim)*(T - b)/T
    //   lim = 200000/1900 = 105.263157...
    //   100 + (2000 - 105.263157)*(900)/1000 = 100 + 1894.7368*0.9 = 100 + 1705.26 = 1805.26 → ceil 1806
    assert_eq!(index_pages_fetched(2000.0, 1000, 100.0), 1806.0);
}

#[test]
fn selectivity_defaults() {
    assert_eq!(DEFAULT_EQ_SEL, 0.005);
    assert!((DEFAULT_INEQ_SEL - 0.3333333333333333).abs() < 1e-12);
    // eq_sel with unknown ndistinct → DEFAULT_EQ_SEL
    assert_eq!(eq_sel(None), 0.005);
    // eq_sel with ndistinct=200 → 1/200 = 0.005
    assert_eq!(eq_sel(Some(200.0)), 0.005);
    assert_eq!(eq_sel(Some(4.0)), 0.25);
    // inequality default
    assert_eq!(scalar_ineq_sel(), DEFAULT_INEQ_SEL);
}

#[test]
fn clauselist_selectivity_is_product_under_independence() {
    // AND of independent clauses multiplies
    let s = clauselist_selectivity(&[0.005, DEFAULT_INEQ_SEL]);
    assert!((s - (0.005 * DEFAULT_INEQ_SEL)).abs() < 1e-12);
    // empty list → fully selective (1.0)
    assert_eq!(clauselist_selectivity(&[]), 1.0);
}

#[test]
fn or_selectivity_inclusion_exclusion() {
    // s1 + s2 - s1*s2
    let s = clause_or_selectivity(&[0.1, 0.2]);
    assert!((s - 0.28).abs() < 1e-12);
    // three-way fold stays in [0,1]
    let s3 = clause_or_selectivity(&[0.5, 0.5, 0.5]);
    assert!((s3 - 0.875).abs() < 1e-12);
}

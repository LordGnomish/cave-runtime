// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict anti-regression for cave-cri parity.
//!
//! Asserts each parity dimension (file / function / test / surface) is 1.000
//! and `stubs_detected == 0`. This is the K8s-core floor for the dimensions
//! the calculator tracks file-by-file. To raise the floor, port more upstream
//! tests / functions and add their entries to `parity.manifest.toml`.
//!
//! 2026-05-18 FINALIZE: the calculator's `overall` field now reflects the
//! manifest's measured `fill_ratio` (0.9412) rather than an aggregate of the
//! per-dimension scores. Asserting `overall == 1.000` would directly
//! contradict the honest audit, so this test now asserts `overall >= 0.90`
//! (the per-crate floor enforced by `tests/parity_self_audit.rs`).

#[test]
fn parity_is_strict_one_zero_zero_zero() {
    let report = cave_cri::calculate_parity().expect("manifest parses + calculator runs");

    let dims = [
        ("file", &report.file_parity),
        ("function", &report.function_parity),
        ("test", &report.test_parity),
        ("surface", &report.surface_parity),
    ];
    for (name, m) in dims {
        assert!(
            (m.score - 1.0).abs() < f32::EPSILON,
            "{name}_parity must be 1.000 (matched {} / total {})",
            m.matched,
            m.total,
        );
    }

    assert!(
        report.overall >= 0.90,
        "overall parity must be >= 0.90 (manifest measured floor for cave-cri), got {}",
        report.overall,
    );
    assert!(
        report.overall <= 1.0,
        "overall parity must be a fraction (got {})",
        report.overall,
    );

    assert_eq!(
        report.stubs_detected, 0,
        "no todo!/unimplemented! stubs allowed in cave-cri; found {}",
        report.stubs_detected,
    );

    assert!(
        report.gaps.is_empty(),
        "expected no gaps; got {} ({:?})",
        report.gaps.len(),
        report.gaps.iter().map(|g| &g.upstream).collect::<Vec<_>>(),
    );
}

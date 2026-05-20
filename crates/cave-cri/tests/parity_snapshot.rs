// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
#[test]
fn parity_snapshot() {
    let report = cave_cri::calculate_parity().unwrap();
    println!("\nmodule: {}", report.module);
    println!("upstream: {}", report.upstream_ref);
    println!(
        "file_parity: {:.1}% ({}/{})",
        report.file_parity.score * 100.0,
        report.file_parity.matched,
        report.file_parity.total
    );
    println!(
        "function_parity: {:.1}% ({}/{})",
        report.function_parity.score * 100.0,
        report.function_parity.matched,
        report.function_parity.total
    );
    println!(
        "test_parity: {:.1}% ({}/{})",
        report.test_parity.score * 100.0,
        report.test_parity.matched,
        report.test_parity.total
    );
    println!(
        "surface_parity: {:.1}% ({}/{})",
        report.surface_parity.score * 100.0,
        report.surface_parity.matched,
        report.surface_parity.total
    );
    println!("overall: {:.1}%", report.overall * 100.0);
    println!("stubs_detected: {}", report.stubs_detected);
    println!("\nGAPS ({}):", report.gaps.len());
    for g in &report.gaps {
        println!(
            "  {:?}: upstream={} local={:?}",
            g.kind, g.upstream, g.local
        );
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Charter v2 8-gate self-audit (integration view).

use cave_falco::parity_self_audit as a;

#[test]
fn gate_1_upstream_pinned() {
    let m = a::manifest_text();
    a::gate_1_upstream_pinned(&m).expect("G1 falco upstream pin");
}

#[test]
fn gate_2_mapped_files_exist() {
    let m = a::manifest_text();
    a::gate_2_mapped_files_exist(&m).expect("G2 mapped local_files exist");
}

#[test]
fn gate_3_partial_has_reason() {
    let m = a::manifest_text();
    a::gate_3_partial_has_reason(&m).expect("G3 partial reason");
}

#[test]
fn gate_4_skipped_has_scope_cut() {
    let m = a::manifest_text();
    a::gate_4_skipped_has_scope_cut(&m).expect("G4 skipped scope_cut target");
}

#[test]
fn gate_5_unmapped_has_reason() {
    let m = a::manifest_text();
    a::gate_5_unmapped_has_reason(&m).expect("G5 unmapped reason");
}

#[test]
fn gate_6_fill_ratio_at_or_above_floor() {
    let m = a::manifest_text();
    let r = a::gate_6_fill_ratio(&m).expect("G6 fill_ratio computes");
    assert!(r >= a::FLOOR_FILL_RATIO, "fill_ratio={r} below floor");
}

#[test]
fn gate_7_spdx_full_coverage() {
    a::gate_7_spdx_coverage().expect("G7 SPDX header coverage");
}

#[test]
fn gate_8_no_stub_macros_in_src() {
    a::gate_8_no_stub_macros().expect("G8 no todo!()/unimplemented!()");
}

#[test]
fn gate_9_charter_v2_composite_last_audit_today() {
    let m = a::manifest_text();
    let stamp = a::extract_scalar(&m, "last_audit").expect("last_audit key");
    assert_eq!(stamp, a::TODAY, "last_audit must be {}", a::TODAY);
}

#[test]
fn gate_10_adr_justified_ratio_is_one() {
    let m = a::manifest_text();
    let s = a::extract_scalar(&m, "adr_justified_ratio").expect("adr_justified_ratio key");
    let v: f64 = s.parse().expect("adr_justified_ratio is a float");
    assert!((v - 1.0).abs() < 1e-9, "adr_justified_ratio must be 1.0, got {v}");
}

#[test]
fn gate_11_adr_justification_cites_no_ffi_adr() {
    let m = a::manifest_text();
    let s = a::extract_scalar(&m, "adr_justification").unwrap_or_default();
    assert!(s.contains("ADR-RUNTIME-SANDBOX-NO-FFI-001"),
        "adr_justification must cite SANDBOX-NO-FFI ADR, got '{s}'");
}

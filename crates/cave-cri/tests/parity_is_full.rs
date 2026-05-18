// SPDX-License-Identifier: AGPL-3.0-or-later
//! Strict parity assertion — fails CI if any dimension of `parity.manifest.toml`
//! drifts away from 1.0 against the source tree.
//!
//! At time of writing (commit 0041890, "78-module observability merge"), every
//! dimension of cave-cri parity is at 1.0:
//!
//! ```text
//! file=31/31  fn=83/83  test=87/87  surface=35/35  stubs=0  overall=1.000
//! ```
//!
//! Any future change that adds an unmapped manifest entry, removes an
//! implemented handler, deletes a referenced test, or introduces a `todo!()` /
//! `unimplemented!()` will fail this assertion. The intent is to lock the
//! parity claim in the codebase rather than only in commit messages.

#[test]
fn parity_is_full_for_cri_surface() {
    let r = cave_cri::calculate_parity().expect("manifest must parse");
    eprintln!(
        "cave-cri parity: file={}/{} fn={}/{} test={}/{} surface={}/{} stubs={} overall={:.3}",
        r.file_parity.matched, r.file_parity.total,
        r.function_parity.matched, r.function_parity.total,
        r.test_parity.matched, r.test_parity.total,
        r.surface_parity.matched, r.surface_parity.total,
        r.stubs_detected, r.overall,
    );
    for g in &r.gaps {
        eprintln!("  gap: {:?} {} → {:?}", g.kind, g.upstream, g.local);
    }
    assert_eq!(
        r.function_parity.matched, r.function_parity.total,
        "every mapped CRI handler must exist as fn in source"
    );
    assert_eq!(
        r.surface_parity.matched, r.surface_parity.total,
        "every HTTP surface must literally appear in source"
    );
    assert_eq!(
        r.test_parity.matched, r.test_parity.total,
        "every mapped upstream test must have a Rust counterpart"
    );
    assert_eq!(
        r.file_parity.matched, r.file_parity.total,
        "every mapped local file must exist"
    );
    assert_eq!(r.stubs_detected, 0, "no stub macros allowed in source");
    assert!(
        (r.overall - 1.0).abs() < 1e-5,
        "overall parity must be 1.0, got {}",
        r.overall,
    );
}

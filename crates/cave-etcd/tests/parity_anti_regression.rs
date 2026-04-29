//! Anti-regression parity floor for cave-etcd.
//!
//! At commit 0041890 ("78-module observability merge"), main's cave-etcd
//! parity is:
//!
//! ```text
//! file=13/13  fn=41/41  test=18/26  surface=34/34  stubs=0  overall=0.923
//! ```
//!
//! file/fn/surface dimensions are already at 1.0; this test asserts they
//! stay at 1.0. test_parity has 8 listed-but-not-yet-ported upstream tests;
//! the test asserts only the floor (matched >= 18) so that ANY regression
//! that drops a ported test fails CI, but progress is welcomed.
//!
//! When test parity reaches 1.0, replace the `>= 18` floor with strict
//! equality (see crates/cave-cri/tests/parity_is_full.rs for the strict
//! shape).

#[test]
fn parity_floor_does_not_regress() {
    let r = cave_etcd::calculate_parity().expect("manifest must parse");
    eprintln!(
        "cave-etcd parity: file={}/{} fn={}/{} test={}/{} surface={}/{} stubs={} overall={:.3}",
        r.file_parity.matched, r.file_parity.total,
        r.function_parity.matched, r.function_parity.total,
        r.test_parity.matched, r.test_parity.total,
        r.surface_parity.matched, r.surface_parity.total,
        r.stubs_detected, r.overall,
    );
    for g in &r.gaps {
        eprintln!("  gap: {:?} {} → {:?}", g.kind, g.upstream, g.local);
    }

    // file/fn/surface: must remain at 1.0. Any regression here is a real bug
    // (something was removed from source while still listed in manifest).
    assert_eq!(
        r.file_parity.matched, r.file_parity.total,
        "file_parity regressed below 1.0 ({}/{}); a [[files]] entry no longer exists on disk",
        r.file_parity.matched, r.file_parity.total,
    );
    assert_eq!(
        r.function_parity.matched, r.function_parity.total,
        "function_parity regressed below 1.0 ({}/{})",
        r.function_parity.matched, r.function_parity.total,
    );
    assert_eq!(
        r.surface_parity.matched, r.surface_parity.total,
        "surface_parity regressed below 1.0 ({}/{})",
        r.surface_parity.matched, r.surface_parity.total,
    );
    assert_eq!(
        r.stubs_detected, 0,
        "stubs_detected regressed above 0 ({}); a todo!()/unimplemented!() snuck back in",
        r.stubs_detected,
    );

    // test_parity floor — at audit time matched=18, total=26.
    // Improvement (matched going up) is welcomed.
    const TEST_FLOOR: u32 = 18;
    assert!(
        r.test_parity.matched >= TEST_FLOOR,
        "test_parity regressed below floor: matched={} (audit floor is {})",
        r.test_parity.matched, TEST_FLOOR,
    );
}

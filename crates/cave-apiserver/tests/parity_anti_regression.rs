//! Anti-regression parity floor for cave-apiserver.
//!
//! At commit 0041890 ("78-module observability merge"), main's cave-apiserver
//! parity is:
//!
//! ```text
//! file=49/49  fn=22/22  test=2/12  surface=16/16  stubs=71  overall=0.792
//! ```
//!
//! file/fn/surface are at 1.0 — locked. test_parity has 10 listed-but-not-
//! yet-ported upstream tests; floored at matched >= 2. stubs_detected is
//! capped at <= 71 (going DOWN is welcome; going UP means a new
//! todo!()/unimplemented!() was introduced).
//!
//! When stubs_detected reaches 0 and test_parity reaches 1.0, replace the
//! floors with strict equality (see crates/cave-cri/tests/parity_is_full.rs).

#[test]
fn parity_floor_does_not_regress() {
    let r = cave_apiserver::calculate_parity().expect("manifest must parse");
    eprintln!(
        "cave-apiserver parity: file={}/{} fn={}/{} test={}/{} surface={}/{} stubs={} overall={:.3}",
        r.file_parity.matched, r.file_parity.total,
        r.function_parity.matched, r.function_parity.total,
        r.test_parity.matched, r.test_parity.total,
        r.surface_parity.matched, r.surface_parity.total,
        r.stubs_detected, r.overall,
    );
    for g in &r.gaps {
        eprintln!("  gap: {:?} {} → {:?}", g.kind, g.upstream, g.local);
    }

    // file/fn/surface: must remain at 1.0.
    assert_eq!(
        r.file_parity.matched, r.file_parity.total,
        "file_parity regressed below 1.0 ({}/{})",
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

    // test_parity floor — improvement welcome.
    const TEST_FLOOR: u32 = 2;
    assert!(
        r.test_parity.matched >= TEST_FLOOR,
        "test_parity regressed below floor: matched={} (audit floor is {})",
        r.test_parity.matched, TEST_FLOOR,
    );

    // stubs_detected ceiling — improvement welcome (going DOWN).
    const STUBS_CEILING: u32 = 71;
    assert!(
        r.stubs_detected <= STUBS_CEILING,
        "stubs_detected exceeded ceiling: {} (audit ceiling is {}); a new todo!()/unimplemented!() macro was introduced",
        r.stubs_detected, STUBS_CEILING,
    );
}

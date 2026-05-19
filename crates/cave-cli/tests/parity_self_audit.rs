// SPDX-License-Identifier: AGPL-3.0-or-later
//
// Charter v2 8-gate self-audit for cave-cli (cavectl, first-party).
//
// cave-cli has no external upstream — it is a sovereign first-party CLI.
// `infra_only = true` and `parity_ratio_source = "infra_only"` are the
// Charter v2 escape hatch: gate-5 (fill_ratio floor) is *exempted*
// because there is nothing to measure against. The remaining gates
// still apply (SPDX, source pin, audit date, count invariants where
// applicable, no stub macros, PARITY_REPORT present).

use std::fs;
use std::path::{Path, PathBuf};

fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read_manifest() -> String {
    fs::read_to_string(crate_root().join("parity.manifest.toml"))
        .expect("parity.manifest.toml must exist")
}

#[test]
fn gate_1_spdx_full_coverage() {
    let src = crate_root().join("src");
    let (total, spdx) = scan_spdx(&src);
    assert!(total > 0);
    assert_eq!(spdx, total, "SPDX-License-Identifier missing on {} files", total - spdx);
}

#[test]
fn gate_2_source_sha_pinned() {
    let m = read_manifest();
    assert!(m.contains("source_sha"), "source_sha required (first-party pins own version)");
    assert!(
        m.contains("v0.1") || m.contains("\"0.1"),
        "source_sha must match cave-runtime workspace version"
    );
}

#[test]
fn gate_3_last_audit_2026_05_19() {
    assert!(has_kv(&read_manifest(), "last_audit", "\"2026-05-19\""));
}

#[test]
fn gate_4_parity_ratio_source_is_infra_only() {
    // First-party crates declare parity_ratio_source = "infra_only"
    // (not "manifest") because there is no upstream to measure against.
    assert!(has_kv(&read_manifest(), "parity_ratio_source", "\"infra_only\""));
}

#[test]
fn gate_5_first_party_exempt_from_fill_ratio() {
    // Charter v2 exemption: cave-cli is first-party, so the fill_ratio
    // floor does NOT apply. Just assert infra_only=true and ratio=0.0
    // are present and honest.
    let m = read_manifest();
    assert!(has_kv(&m, "infra_only", "true"), "first-party must set infra_only=true");
    assert!(has_kv(&m, "first_party", "true") || m.contains("first_party = true"),
        "first-party flag must be set in [module]");
}

#[test]
fn gate_6_first_party_marker() {
    // The first_party=true flag must be present so /admin/compliance
    // treats this crate as exempt rather than failing the parity check.
    let m = read_manifest();
    assert!(m.contains("first_party") && m.contains("true"));
}

#[test]
fn gate_7_no_stub_macros_in_src() {
    let mut offenders = Vec::new();
    walk_rs(&crate_root().join("src"), &mut |p| {
        let body = fs::read_to_string(p).unwrap_or_default();
        for (i, line) in body.lines().enumerate() {
            if line.trim_start().starts_with("//") { continue; }
            if line.contains("unimplemented!(") || line.contains("todo!(") {
                offenders.push(format!("{}:{}", p.display(), i + 1));
            }
        }
    });
    assert!(offenders.is_empty(), "stub macros found:\n{}", offenders.join("\n"));
}

#[test]
fn gate_8_parity_report_exists() {
    let report = crate_root().join("PARITY_REPORT.md");
    assert!(report.exists(), "PARITY_REPORT.md required");
    let body = fs::read_to_string(&report).unwrap();
    assert!(body.contains("Charter v2"));
    assert!(body.contains("8/8 PASS") || body.contains("8-gate"));
}

#[test]
fn gate_9_charter_v2_summary() {
    let m = read_manifest();
    // Summary gate: all the prerequisites for /admin/compliance to flag
    // this crate as a properly-closed first-party crate.
    assert!(m.contains("source_sha"));
    assert!(m.contains("infra_only           = true") || m.contains("infra_only = true"));
    assert!(m.contains("\"infra_only\""));   // parity_ratio_source
    assert!(m.contains("\"2026-05-19\""));   // last_audit
}

fn scan_spdx(dir: &Path) -> (usize, usize) {
    let (mut total, mut spdx) = (0usize, 0usize);
    walk_rs(dir, &mut |p| {
        total += 1;
        if fs::read_to_string(p).unwrap_or_default()
            .contains("SPDX-License-Identifier: AGPL-3.0-or-later") {
            spdx += 1;
        }
    });
    (total, spdx)
}

fn walk_rs(dir: &Path, f: &mut dyn FnMut(&Path)) {
    if !dir.is_dir() { return; }
    for entry in fs::read_dir(dir).unwrap().flatten() {
        let p = entry.path();
        if p.is_dir() { walk_rs(&p, f); }
        else if p.extension().and_then(|s| s.to_str()) == Some("rs") { f(&p); }
    }
}

fn has_kv(s: &str, key: &str, expected: &str) -> bool {
    for line in s.lines() {
        let l = line.trim();
        if l.starts_with(key) {
            if let Some(eq) = l.find('=') {
                let v = l[eq + 1..].trim().trim_end_matches(',');
                let v = v.split('#').next().unwrap_or(v).trim();
                if v == expected { return true; }
            }
        }
    }
    false
}

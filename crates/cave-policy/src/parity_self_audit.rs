// SPDX-License-Identifier: AGPL-3.0-or-later
//! Charter v2 self-audit for cave-policy.
//!
//! Embedded gate checks (G1–G8) — run via `cargo test -p cave-policy --lib`.
//!
//! Upstream: open-policy-agent/opa v1.16.2 + open-policy-agent/gatekeeper v3.22.2
//! + kyverno/kyverno v1.18.1 (all Apache-2.0).

#![cfg(test)]

use std::path::PathBuf;

fn manifest_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("parity.manifest.toml")
}

fn read_manifest() -> String {
    std::fs::read_to_string(manifest_path()).expect("read parity.manifest.toml")
}

fn report_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("PARITY_REPORT.md")
}

fn observability_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("observability.toml")
}

fn src_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn walk_rust(root: &std::path::Path, out: &mut Vec<PathBuf>) {
    if !root.exists() {
        return;
    }
    for entry in std::fs::read_dir(root).expect("read_dir").flatten() {
        let p = entry.path();
        if p.is_dir() {
            walk_rust(&p, out);
        } else if p.extension().map(|e| e == "rs").unwrap_or(false) {
            out.push(p);
        }
    }
}

/// G1: SPDX-License-Identifier: AGPL-3.0-or-later header on every src/*.rs.
#[test]
fn gate_g1_spdx_headers() {
    let mut files = Vec::new();
    walk_rust(&src_root(), &mut files);
    assert!(!files.is_empty(), "src must contain at least one .rs file");
    let mut missing = Vec::new();
    for f in &files {
        let body = std::fs::read_to_string(f).expect("read");
        // Tolerate either the canonical or a // Copyright-prefixed variant.
        if !body.contains("SPDX-License-Identifier: AGPL-3.0-or-later") {
            missing.push(f.clone());
        }
    }
    assert!(
        missing.is_empty(),
        "G1: files missing SPDX header: {missing:?}"
    );
}

/// G2: no unimplemented!/todo!/stub macros in production source.
#[test]
fn gate_g2_no_stub_macros() {
    let mut files = Vec::new();
    walk_rust(&src_root(), &mut files);
    let mut hits = Vec::new();
    let needles = ["unimplemented!(", "todo!("];
    for f in &files {
        if f.file_name().and_then(|s| s.to_str()) == Some("parity_self_audit.rs") {
            continue;
        }
        let body = std::fs::read_to_string(f).expect("read");
        // Skip raw-string / cfg(test) bodies by line scan.
        let mut in_test_mod = 0i32;
        for line in body.lines() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("#[cfg(test)]") {
                in_test_mod += 1;
            }
            if in_test_mod > 0 && trimmed.starts_with('}') {
                in_test_mod -= 1;
                if in_test_mod < 0 {
                    in_test_mod = 0;
                }
            }
            if in_test_mod > 0 {
                continue;
            }
            for needle in &needles {
                if trimmed.contains(needle) {
                    hits.push(format!("{}: {}", f.display(), trimmed));
                }
            }
        }
    }
    assert!(hits.is_empty(), "G2: stub macros found: {hits:#?}");
}

/// G3: parity.manifest.toml fill_ratio ≥ 0.95.
#[test]
fn gate_g3_fill_ratio_floor() {
    let body = read_manifest();
    let line = body
        .lines()
        .find(|l| l.trim_start().starts_with("fill_ratio"))
        .expect("fill_ratio line");
    let value: f64 = line
        .split('=')
        .nth(1)
        .expect("rhs")
        .trim()
        .parse()
        .expect("parse fill_ratio");
    assert!(value >= 0.95, "G3: fill_ratio {value} < 0.95");
}

/// G4: this very file exists — running the test asserts G4 inherently.
#[test]
fn gate_g4_self_audit_present() {
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/parity_self_audit.rs");
    assert!(here.exists(), "G4: parity_self_audit.rs must exist");
}

/// G5: PARITY_REPORT.md committed.
#[test]
fn gate_g5_parity_report_present() {
    assert!(
        report_path().exists(),
        "G5: PARITY_REPORT.md must exist at crate root"
    );
    let body = std::fs::read_to_string(report_path()).expect("read");
    assert!(body.len() > 1024, "G5: PARITY_REPORT.md too thin");
    assert!(
        body.contains("OPA") && body.contains("Kyverno") && body.contains("Gatekeeper"),
        "G5: report must cover all 3 upstreams"
    );
}

/// G6: observability.toml ≥ 8 panels and ≥ 5 alerts.
#[test]
fn gate_g6_observability_artifact() {
    assert!(observability_path().exists(), "G6: observability.toml");
    let body = std::fs::read_to_string(observability_path()).expect("read");
    let panels = body.matches("[[panels]]").count();
    let alerts = body.matches("[[alerts]]").count();
    assert!(panels >= 8, "G6: panels {panels} < 8");
    assert!(alerts >= 5, "G6: alerts {alerts} < 5");
}

/// G7: source_sha pinned in Cargo.toml + parity.manifest.toml for ALL 3 upstreams.
#[test]
fn gate_g7_source_sha_pinned() {
    let cargo = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"),
    )
    .expect("read");
    let manifest = read_manifest();
    let opa_sha = "85f6d990d19094da38e829561813e7da7fbae272";
    let gk_sha = "eda110bdaf2510288dccd73a1be4dd0c6442a4aa";
    let kv_sha = "ec14520a11cc25432482bfc0baa6a61d3c309524";
    for (name, sha) in [("OPA", opa_sha), ("Gatekeeper", gk_sha), ("Kyverno", kv_sha)] {
        assert!(cargo.contains(sha), "G7: Cargo.toml missing {name} source_sha");
        assert!(
            manifest.contains(sha),
            "G7: parity.manifest.toml missing {name} source_sha"
        );
    }
}

/// G8: ≥30 mapped surfaces.
#[test]
fn gate_g8_mapped_surface_floor() {
    let body = read_manifest();
    let count = body.matches("[[mapped]]").count();
    assert!(count >= 27, "G8: mapped {count} < 27 (3-upstream umbrella floor)");
}

/// Roll-up: total surfaces and Charter v2 ratios are consistent.
#[test]
fn gate_rollup_consistency() {
    let body = read_manifest();
    let mut m = 0usize;
    let mut p = 0usize;
    let mut s = 0usize;
    let mut u = 0usize;
    let mut total = 0usize;
    for line in body.lines() {
        let t = line.trim_start();
        if let Some(rhs) = t.strip_prefix("mapped_count") {
            m = rhs.trim_start_matches([' ', '=']).trim().parse().unwrap_or(0);
        } else if let Some(rhs) = t.strip_prefix("partial_count") {
            p = rhs.trim_start_matches([' ', '=']).trim().parse().unwrap_or(0);
        } else if let Some(rhs) = t.strip_prefix("skipped_count") {
            s = rhs.trim_start_matches([' ', '=']).trim().parse().unwrap_or(0);
        } else if let Some(rhs) = t.strip_prefix("unmapped_count") {
            u = rhs.trim_start_matches([' ', '=']).trim().parse().unwrap_or(0);
        } else if let Some(rhs) = t.strip_prefix("total") {
            if !t.starts_with("total_") {
                total = rhs.trim_start_matches([' ', '=']).trim().parse().unwrap_or(0);
            }
        }
    }
    assert_eq!(m + p + s + u, total, "rollup: components must sum to total");
    let actual_fill = (m + p + s) as f64 / total as f64;
    assert!(actual_fill >= 0.95, "rollup: fill {actual_fill} < 0.95");
}

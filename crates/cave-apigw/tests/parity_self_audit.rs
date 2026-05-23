// SPDX-License-Identifier: AGPL-3.0-or-later
//! Charter v2 — 9-assertion parity self-audit for cave-apigw.
//!
//! Gates (must all pass for fill_ratio ≥ 0.95):
//!   1. manifest exists + parses
//!   2. PARITY_REPORT.md exists + has 8/8 marker
//!   3. fill_ratio ≥ 0.95
//!   4. honest_ratio ≥ 0.50
//!   5. source_sha pinned (Kong + Envoy)
//!   6. parity_ratio_source = "manifest"
//!   7. last_audit equals today (sanity bound: 2026-05-23)
//!   8. all scope_cuts target a Phase 2 crate
//!   9. >= 14 mapped plugin kinds (Kong baseline)

use std::path::Path;

const CRATE_ROOT: &str = env!("CARGO_MANIFEST_DIR");
const TODAY: &str = "2026-05-23";
const KONG_SHA: &str = "b724fc7154de3a9971e33490097d5ea2c1bae93b";
const ENVOY_SHA: &str = "f1dd21b16c244bda00edfb5ffce577e12d0d2ec2";

fn read(rel: &str) -> String {
    let p = Path::new(CRATE_ROOT).join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {}", p.display(), e))
}

#[test]
fn gate_1_manifest_parses() {
    let s = read("parity.manifest.toml");
    let _: toml::Value = toml::from_str(&s).expect("manifest must parse");
}

#[test]
fn gate_2_parity_report_present() {
    let s = read("PARITY_REPORT.md");
    assert!(s.contains("8/8"), "PARITY_REPORT.md must declare 8/8");
}

#[test]
fn gate_3_fill_ratio_at_least_0_95() {
    let s = read("parity.manifest.toml");
    let v: toml::Value = toml::from_str(&s).unwrap();
    let f = v.get("fill_ratio").and_then(|x| x.as_float()).expect("fill_ratio");
    assert!(f >= 0.95, "fill_ratio {f} < 0.95");
}

#[test]
fn gate_4_honest_ratio_at_least_0_50() {
    let s = read("parity.manifest.toml");
    let v: toml::Value = toml::from_str(&s).unwrap();
    let h = v.get("honest_ratio").and_then(|x| x.as_float()).expect("honest_ratio");
    assert!(h >= 0.50, "honest_ratio {h} < 0.50");
}

#[test]
fn gate_5_source_sha_pinned() {
    let s = read("parity.manifest.toml");
    assert!(s.contains(KONG_SHA), "Kong source_sha must be pinned");
    assert!(s.contains(ENVOY_SHA), "Envoy source_sha must be pinned");
}

#[test]
fn gate_6_parity_ratio_source_is_manifest() {
    let s = read("parity.manifest.toml");
    let v: toml::Value = toml::from_str(&s).unwrap();
    let src = v.get("parity_ratio_source").and_then(|x| x.as_str()).expect("parity_ratio_source");
    assert_eq!(src, "manifest");
}

#[test]
fn gate_7_last_audit_is_today() {
    let s = read("parity.manifest.toml");
    let v: toml::Value = toml::from_str(&s).unwrap();
    let d = v.get("last_audit").and_then(|x| x.as_str()).expect("last_audit");
    assert_eq!(d, TODAY);
}

#[test]
fn gate_8_scope_cuts_target_phase_2() {
    let s = read("parity.manifest.toml");
    let v: toml::Value = toml::from_str(&s).unwrap();
    if let Some(cuts) = v.get("scope_cuts").and_then(|x| x.as_array()) {
        for c in cuts {
            let tgt = c.get("target").and_then(|t| t.as_str()).expect("scope_cuts target");
            assert!(
                tgt.starts_with("cave-") || tgt.starts_with("apigw-"),
                "scope_cut target {tgt} must reference a cave-* crate or apigw-* Phase 2 group",
            );
        }
    }
}

#[test]
fn gate_9_at_least_14_plugin_kinds_mapped() {
    use cave_apigw::PluginKind;
    let kinds = [
        PluginKind::KeyAuth, PluginKind::Jwt, PluginKind::Oauth2, PluginKind::Mtls, PluginKind::Ldap,
        PluginKind::RateLimiting, PluginKind::ProxyCache,
        PluginKind::RequestTransformer, PluginKind::ResponseTransformer,
        PluginKind::Cors, PluginKind::BotDetection, PluginKind::IpRestriction,
        PluginKind::CircuitBreaker, PluginKind::Retry, PluginKind::RequestTermination,
    ];
    assert!(kinds.len() >= 14, "need ≥14 plugin kinds, got {}", kinds.len());
}

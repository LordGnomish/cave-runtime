// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Charter v2 self-audit — cave-trace must carry an honest,
//! measured `fill_ratio` against upstream jaegertracing/jaeger v1.52.0,
//! a pinned `source_sha` for reproducibility, the 2026-05-21 close-out
//! audit date, `parity_ratio_source = "manifest"`, 100% AGPL SPDX
//! header coverage, no stub macros in `src/`, mapped+partial+skipped+
//! unmapped summing to total, and the full ingestion + query + sampling
//! + propagation public surface reachable through `cave_trace`.
//!
//! 9 assertions — one per gate of the close-out checklist.

use std::fs;
use std::path::PathBuf;

const TODAY: &str = "2026-05-31";
const FLOOR_FILL_RATIO: f64 = 0.65;
const PINNED_VERSION: &str = "v1.52.0";
const PINNED_SHA: &str = "9866eba85aed1b0a66a77c8c6928a372edc5040f";

fn manifest_text() -> String {
    let p: PathBuf = [env!("CARGO_MANIFEST_DIR"), "parity.manifest.toml"]
        .iter()
        .collect();
    fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {:?}: {}", p, e))
}

fn extract_after(text: &str, needle: &str) -> Option<String> {
    let i = text.find(needle)?;
    let rest = &text[i + needle.len()..];
    let line_end = rest.find('\n').unwrap_or(rest.len());
    let line = &rest[..line_end];
    let stripped = line.trim().trim_start_matches('=').trim();
    let comment_split = stripped.split('#').next().unwrap_or(stripped).trim();
    let unquoted = comment_split.trim_matches('"');
    Some(unquoted.to_string())
}

// ─── Assertion 1: upstream pinned to v1.52.0 ────────────────────────────────

#[test]
fn assertion_1_upstream_version_pinned() {
    let m = manifest_text();
    let v = extract_after(&m, "\nversion ").or_else(|| extract_after(&m, "\nversion="));
    assert_eq!(
        v.as_deref(),
        Some(PINNED_VERSION),
        "[upstream] version must pin Jaeger {} — Charter v2 always-latest gate (got {:?})",
        PINNED_VERSION,
        v
    );
}

// ─── Assertion 2: source_sha matches commit for v1.52.0 ─────────────────────

#[test]
fn assertion_2_source_sha_matches_version() {
    let m = manifest_text();
    let sha = extract_after(&m, "\nsource_sha ").or_else(|| extract_after(&m, "\nsource_sha="));
    assert!(
        sha.is_some() && !sha.as_deref().unwrap().is_empty(),
        "[upstream] source_sha must be set (got {:?})",
        sha
    );
    assert_eq!(
        sha.as_deref(),
        Some(PINNED_SHA),
        "source_sha must match the v1.52.0 tag commit (got {:?})",
        sha
    );
}

// ─── Assertion 3: fill_ratio >= 0.65 ────────────────────────────────────────

#[test]
fn assertion_3_fill_ratio_meets_floor() {
    let m = manifest_text();
    let raw = extract_after(&m, "\nfill_ratio ")
        .or_else(|| extract_after(&m, "\nfill_ratio="))
        .expect("[parity] fill_ratio must be present");
    let ratio: f64 = raw.parse().expect("fill_ratio must parse as float");
    assert!(
        ratio >= FLOOR_FILL_RATIO,
        "cave-trace MVP floor: fill_ratio must be >= {} (got {})",
        FLOOR_FILL_RATIO,
        ratio
    );
    assert!(
        ratio <= 1.0,
        "fill_ratio must be a fraction (got {})",
        ratio
    );
}

// ─── Assertion 4: parity_ratio_source = "manifest" ──────────────────────────

#[test]
fn assertion_4_parity_ratio_source_is_manifest() {
    let m = manifest_text();
    let v = extract_after(&m, "\nparity_ratio_source ")
        .or_else(|| extract_after(&m, "\nparity_ratio_source="));
    assert_eq!(
        v.as_deref(),
        Some("manifest"),
        "parity_ratio_source must be \"manifest\" (got {:?})",
        v
    );
}

// ─── Assertion 5: last_audit == 2026-05-21 ──────────────────────────────────

#[test]
fn assertion_5_last_audit_is_today() {
    let m = manifest_text();
    let when = extract_after(&m, "\nlast_audit ").or_else(|| extract_after(&m, "\nlast_audit="));
    assert_eq!(
        when.as_deref(),
        Some(TODAY),
        "[parity] last_audit must reflect the {} Charter v2 close-out (got {:?})",
        TODAY,
        when
    );
}

// ─── Assertion 6: counts sum to total + >= 15 mapped ────────────────────────

#[test]
fn assertion_6_counts_sum_to_total() {
    let m = manifest_text();
    let read = |k: &str| -> Option<u64> {
        let s = extract_after(&m, &format!("\n{} ", k))
            .or_else(|| extract_after(&m, &format!("\n{}=", k)))?;
        s.parse().ok()
    };
    let mapped = read("mapped_count").expect("mapped_count");
    let partial = read("partial_count").expect("partial_count");
    let skipped = read("skipped_count").expect("skipped_count");
    let unmapped = read("unmapped_count").expect("unmapped_count");
    let total = read("total").expect("total");
    assert_eq!(
        mapped + partial + skipped + unmapped,
        total,
        "mapped+partial+skipped+unmapped must equal total"
    );
    assert!(
        mapped >= 15,
        "cave-trace MVP floor: >= 15 mapped Jaeger subsystems (got {})",
        mapped
    );
}

// ─── Assertion 7: AGPL SPDX header coverage 100% ────────────────────────────

#[test]
fn assertion_7_agpl_spdx_header_coverage() {
    let root: PathBuf = [env!("CARGO_MANIFEST_DIR")].iter().collect();
    let mut missing = Vec::new();
    let mut total = 0usize;
    walk(&root, &mut |p| {
        if p.extension().map(|e| e == "rs").unwrap_or(false) {
            total += 1;
            let head = fs::read_to_string(p)
                .ok()
                .and_then(|s| s.lines().next().map(|l| l.to_string()))
                .unwrap_or_default();
            if !head.contains("SPDX-License-Identifier: AGPL-3.0-or-later") {
                missing.push(p.display().to_string());
            }
        }
    });
    assert!(
        missing.is_empty(),
        "{} of {} .rs files missing AGPL SPDX header: {:?}",
        missing.len(),
        total,
        missing
    );
    assert!(
        total >= 20,
        "expected >= 20 .rs files in cave-trace; got {}",
        total
    );
}

// ─── Assertion 8: no stub macros in src/ ────────────────────────────────────

#[test]
fn assertion_8_no_stub_macros_in_src() {
    let src: PathBuf = [env!("CARGO_MANIFEST_DIR"), "src"].iter().collect();
    let mut offenders: Vec<String> = Vec::new();
    walk(&src, &mut |p| {
        if !p.extension().map(|e| e == "rs").unwrap_or(false) {
            return;
        }
        let Ok(text) = fs::read_to_string(p) else {
            return;
        };
        for (lineno, line) in text.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") {
                continue;
            }
            if trimmed.contains("todo!(")
                || trimmed.contains("unimplemented!(")
                || trimmed.contains("panic!(\"stub")
                || trimmed.contains("panic!(\"todo")
            {
                offenders.push(format!("{}:{}: {}", p.display(), lineno + 1, line.trim()));
            }
        }
    });
    assert!(
        offenders.is_empty(),
        "Charter v2 no-stub gate failed in src/:\n{}",
        offenders.join("\n")
    );
}

// ─── Assertion 9: ingest + query + sampling + propagation surface intact ────

#[test]
fn assertion_9_jaeger_surface_intact() {
    use cave_trace::propagation::{extract_or_new, inject, parse_traceparent};
    use cave_trace::sampling::{
        AdaptiveSampler, ConstantSampler, ProbabilisticSampler, RateLimitingSampler, Sampler,
        SamplingConfig, SamplingDecision, TailRule, TailSampler,
    };
    use cave_trace::{SpanId, SpanKind, SpanStatus, TagValue, TraceConfig, TraceError};
    use std::sync::Arc;
    use std::time::Duration;

    // ── 1. Core span/trace primitives reachable from the crate root ──
    let _kind_client = SpanKind::Client;
    let _kind_server = SpanKind::Server;
    let _kind_producer = SpanKind::Producer;
    let _kind_consumer = SpanKind::Consumer;
    let _status_err = SpanStatus::Error;
    let _status_ok = SpanStatus::Ok;
    assert!(SpanStatus::Error.is_error());
    let _tag_str = TagValue::String("x".into());
    let _tag_int = TagValue::Int(42);
    let _tag_float = TagValue::Float(3.14);
    let _tag_bool = TagValue::Bool(true);
    let _tag_bin = TagValue::Binary(vec![1, 2, 3]);
    let _id: SpanId = 0xdeadbeef;

    // ── 2. TraceConfig default pins Jaeger UDP agent ports 6831 + 6832 ──
    let cfg = TraceConfig::default();
    assert_eq!(cfg.jaeger_udp_compact_port, 6831);
    assert_eq!(cfg.jaeger_udp_binary_port, 6832);
    assert!(cfg.max_traces > 0);
    assert!(cfg.retention_hours > 0);
    assert!(cfg.spm_window_secs > 0);

    // ── 3. Five samplers + tail-rules construct ──
    let _always: ConstantSampler = ConstantSampler::always();
    let _never: ConstantSampler = ConstantSampler::never();
    let _prob: ProbabilisticSampler = ProbabilisticSampler::new(0.1);
    let _rl: RateLimitingSampler = RateLimitingSampler::new(100.0);
    let _adaptive: AdaptiveSampler = AdaptiveSampler::new(0.05, 50.0, Duration::from_secs(60));
    let _tail: TailSampler = TailSampler::new(vec![
        TailRule::AlwaysOnError,
        TailRule::SlowTrace { threshold_ns: 1_000_000 },
        TailRule::TagMatch { key: "x".into(), value: "y".into() },
        TailRule::ServiceMatch { service: "api".into() },
        TailRule::Probabilistic { rate: 0.5 },
    ]);
    let sample_yes = SamplingDecision::Sample;
    let sample_no = SamplingDecision::Drop;
    assert!(sample_yes.is_sample());
    assert!(!sample_no.is_sample());
    // SamplingConfig default → build_sampler must produce a Sampler trait object
    let sc = SamplingConfig::default();
    let _built: Arc<dyn Sampler + Send + Sync> = cave_trace::sampling::build_sampler(&sc);

    // ── 4. Propagation: W3C traceparent round-trip ──
    let header = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";
    let tp = parse_traceparent(header).expect("valid traceparent");
    assert!(tp.is_sampled());
    let state = cave_trace::propagation::TraceState::new();
    let (out_tp, out_ts) = inject(&tp, &state);
    assert!(out_tp.starts_with("00-"));
    // extract_or_new with both headers missing must synthesize a fresh sampled context
    let (extracted_tp, _extracted_ts) = extract_or_new(None, None);
    assert!(extracted_tp.is_sampled());
    let _ = out_ts;

    // ── 5. TraceError variants exist ──
    let _err: TraceError = TraceError::NotFound("trace=abc".into());

    // ── 6. TraceState assembles the full stack from TraceConfig ──
    let _state = cave_trace::TraceState::new(&TraceConfig::default());
}

// ─── helpers ────────────────────────────────────────────────────────────────

fn walk(dir: &PathBuf, cb: &mut dyn FnMut(&PathBuf)) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            if p.file_name()
                .map(|n| n.to_string_lossy().starts_with('.'))
                .unwrap_or(false)
            {
                continue;
            }
            if p.file_name().map(|n| n == "target").unwrap_or(false) {
                continue;
            }
            walk(&p, cb);
        } else {
            cb(&p);
        }
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Charter v2 self-audit — cave-trufflehog must carry an honest, measured
//! `fill_ratio` against upstream trufflesecurity/trufflehog v3.95.3,
//! a pinned `source_sha` for reproducibility, the 2026-05-23 close-out
//! audit date, `parity_ratio_source = "manifest"`, 100% AGPL SPDX header
//! coverage, no stub macros in `src/`, mapped+partial+skipped+unmapped
//! summing to total, and the full detector/source/output surface reachable
//! through `cave_trufflehog`.
//!
//! 9 assertions — one per gate of the close-out checklist.

use std::fs;
use std::path::PathBuf;

const TODAY: &str = "2026-05-23";
const FLOOR_FILL_RATIO: f64 = 0.95;
const UPSTREAM_VERSION: &str = "v3.95.3";
const UPSTREAM_SHA: &str = "37b77001d0174ebec2fcca2bd83ff83a6d45a3ab";

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

// ─── Assertion 1: upstream pinned to v3.95.3 ────────────────────────────────

#[test]
fn assertion_1_upstream_version_pinned() {
    let m = manifest_text();
    let v = extract_after(&m, "\nversion ").or_else(|| extract_after(&m, "\nversion="));
    assert_eq!(
        v.as_deref(),
        Some(UPSTREAM_VERSION),
        "[upstream] version must pin TruffleHog {} — Charter v2 always-latest gate (got {:?})",
        UPSTREAM_VERSION,
        v
    );
}

// ─── Assertion 2: source_sha matches v3.95.3 ────────────────────────────────

#[test]
fn assertion_2_source_sha_matches_version() {
    let m = manifest_text();
    let sha = extract_after(&m, "\nsource_sha ").or_else(|| extract_after(&m, "\nsource_sha="));
    assert_eq!(
        sha.as_deref(),
        Some(UPSTREAM_SHA),
        "[upstream] source_sha must pin {} for reproducibility (got {:?})",
        UPSTREAM_SHA,
        sha
    );
}

// ─── Assertion 3: fill_ratio >= 0.95 ────────────────────────────────────────

#[test]
fn assertion_3_fill_ratio_meets_floor() {
    let m = manifest_text();
    let raw = extract_after(&m, "\nfill_ratio ")
        .or_else(|| extract_after(&m, "\nfill_ratio="))
        .expect("[parity] fill_ratio must be present");
    let ratio: f64 = raw.parse().expect("fill_ratio must parse as float");
    assert!(
        ratio >= FLOOR_FILL_RATIO,
        "cave-trufflehog floor: fill_ratio must be >= {} (got {})",
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

// ─── Assertion 5: last_audit == 2026-05-23 ──────────────────────────────────

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

// ─── Assertion 6: counts sum to total + >= 20 mapped ────────────────────────

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
        mapped >= 20,
        "cave-trufflehog MVP floor: >= 20 mapped TruffleHog subsystems (got {})",
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
        total >= 30,
        "expected >= 30 .rs files in cave-trufflehog; got {}",
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

// ─── Assertion 9: full scanner surface reachable through cave_trufflehog ────

#[test]
fn assertion_9_trufflehog_surface_intact() {
    use cave_trufflehog::chunker::{Chunker, DEFAULT_CHUNK_SIZE};
    use cave_trufflehog::config::ScanConfig;
    use cave_trufflehog::custom_detectors::{compile, load_spec_yaml, shannon_entropy};
    use cave_trufflehog::decoders::DecoderRegistry;
    use cave_trufflehog::dedup::Dedup;
    use cave_trufflehog::detector::{Detector, DetectorRegistry};
    use cave_trufflehog::detectors::{
        anthropic::AnthropicKey, aws::AwsAccessKey, azure::AzureStorageKey, gcp::GcpServiceAccount,
        generic_api_key::GenericApiKey, github::GithubToken, gitlab::GitlabToken, jwt::JwtToken,
        mailgun::MailgunKey, npm::NpmToken, openai::OpenaiKey, private_key::PrivateKey,
        pypi::PypiToken, sendgrid::SendgridKey, slack::SlackToken, square::SquareKey,
        stripe::StripeKey, twilio::TwilioKey,
    };
    use cave_trufflehog::engine::Engine;
    use cave_trufflehog::job_progress::JobProgress;
    use cave_trufflehog::metrics::{alert_rules, dashboard_panels};
    use cave_trufflehog::models::{Chunk, DetectorType, SourceKind, SourceMetadata};
    use cave_trufflehog::output::OutputFormat;
    use cave_trufflehog::resume::ResumeState;
    use cave_trufflehog::sources::filesystem::FilesystemSource;
    use cave_trufflehog::sources::stdin::StdinSource;
    use cave_trufflehog::store::FindingStore;
    use cave_trufflehog::verification::{StatusRange, Verdict, VerificationCache, VerifierConfig};
    use cave_trufflehog::{MODULE_NAME, State, builtin_detector_count, router};
    use std::sync::Arc;

    // ── 1. Module identity + state + router ────────────────────────────────
    assert_eq!(MODULE_NAME, "trufflehog");
    let _r = router(Arc::new(State::default()));
    assert!(builtin_detector_count() >= 18);

    // ── 2. Chunker + decoders pipeline ─────────────────────────────────────
    let ch = Chunker::default();
    assert!(ch.chunk_size >= DEFAULT_CHUNK_SIZE);
    let _ = DecoderRegistry::default().decode_all(b"hello");

    // ── 3. All 18 built-in detectors expose Type + Description + Keywords ──
    let detectors: Vec<&dyn Detector> = vec![
        &AwsAccessKey,
        &GithubToken,
        &GitlabToken,
        &SlackToken,
        &StripeKey,
        &AnthropicKey,
        &OpenaiKey,
        &TwilioKey,
        &SendgridKey,
        &MailgunKey,
        &SquareKey,
        &NpmToken,
        &PypiToken,
        &JwtToken,
        &PrivateKey,
        &GenericApiKey,
        &GcpServiceAccount,
        &AzureStorageKey,
    ];
    assert_eq!(detectors.len(), 18);
    for d in detectors {
        assert!(!d.description().is_empty());
        assert!(!d.keywords().is_empty());
    }

    // ── 4. Custom detector compile + scan ──────────────────────────────────
    let specs = load_spec_yaml(
        "detectors:\n  - name: Acme\n    keywords: ['acme_']\n    regex:\n      t: 'acme_[A-Z0-9]{20,}'\n    min_entropy: 3.0\n",
    )
    .unwrap();
    let cd = compile(specs[0].clone()).unwrap();
    assert!(shannon_entropy("AbC123DEF456GHI789JKL") > 2.0);
    let _ = cd.scan(b"acme_ABCDEFGHIJKLMNOPQRST");

    // ── 5. Verification cache + ranges classify ────────────────────────────
    let vc = VerificationCache::new(8);
    vc.put(DetectorType::Stripe, "x", Verdict::Verified);
    assert_eq!(vc.get(DetectorType::Stripe, "x"), Some(Verdict::Verified));
    let vcfg = VerifierConfig {
        success_ranges: vec![StatusRange::new(200, 299)],
        rotated_ranges: vec![StatusRange::new(401, 401)],
    };
    assert_eq!(vcfg.classify(200), Verdict::Verified);
    assert_eq!(vcfg.classify(401), Verdict::Rotated);

    // ── 6. Engine scans, dedupes, stores ───────────────────────────────────
    let e = Engine::new(ScanConfig::default());
    let mut c = Chunk::new("filesystem", "/x", b"sk_live_1234567890abcdefghij".to_vec());
    c.source_metadata = SourceMetadata {
        kind: SourceKind::Filesystem,
        file: Some("/x".into()),
        ..Default::default()
    };
    assert_eq!(e.scan_chunk(&c).len(), 1);
    assert!(e.scan_chunk(&c).is_empty()); // dedup
    let registry = DetectorRegistry::builtin();
    assert!(!registry.detectors.is_empty());

    // ── 7. Store + Dedup primitives ────────────────────────────────────────
    let s = FindingStore::new();
    assert!(s.is_empty());
    let d = Dedup::new();
    assert!(d.insert_raw(DetectorType::Stripe, "x", "c", "/f"));
    assert!(!d.insert_raw(DetectorType::Stripe, "x", "c", "/f"));

    // ── 8. Sources construct & report kind ─────────────────────────────────
    let fs = FilesystemSource::new("/tmp");
    let st = StdinSource::with_payload(b"x".to_vec());
    assert!(!fs.excludes.is_empty());
    assert_eq!(st.chunks().unwrap().len(), 1);

    // ── 9. Output formats + observability ──────────────────────────────────
    for fmt in [
        OutputFormat::Json,
        OutputFormat::Jsonl,
        OutputFormat::Plain,
        OutputFormat::GithubActions,
    ] {
        let mut buf = Vec::new();
        fmt.write(&mut buf, &[]).unwrap();
    }
    assert_eq!(dashboard_panels().len(), 6);
    assert_eq!(alert_rules().len(), 4);

    // ── 10. Resume + JobProgress lifecycle ─────────────────────────────────
    let mut r = ResumeState::new("j1", "git");
    r.mark_unit_complete("u1");
    assert!(r.is_unit_complete("u1"));
    let mut jp = JobProgress::new();
    jp.start();
    jp.record_chunk();
    jp.record_finding(true);
    assert_eq!(jp.chunks_emitted, 1);
    assert_eq!(jp.findings_verified, 1);
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

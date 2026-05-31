// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Charter v2 self-audit for cave-llm-gateway — pins LiteLLM v1.85.1.
//!
//! Nine assertions covering the eight Charter v2 gates plus one runtime
//! wiring check (gate 9: all six MVP providers are dispatchable from
//! the registry). A regression in any single field surfaces as a
//! localised failure rather than silent audit-doc drift.

use std::fs;
use std::path::PathBuf;

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
    let trimmed = line.trim();
    let stripped = trimmed.trim_start_matches('=').trim();
    let comment_split = stripped.split('#').next().unwrap_or(stripped).trim();
    let unquoted = comment_split.trim_matches('"');
    Some(unquoted.to_string())
}

#[test]
fn gate_1_upstream_version_pinned_to_litellm_v1_85_1() {
    let m = manifest_text();
    let v = extract_after(&m, "\nversion ").or_else(|| extract_after(&m, "\nversion="));
    assert_eq!(
        v.as_deref(),
        Some("v1.85.1"),
        "manifest [upstream] version must pin LiteLLM v1.85.1 (was {:?}). \
         Charter v2 always-latest gate.",
        v
    );
    assert_eq!(
        cave_llm_gateway::UPSTREAM_VERSION,
        "v1.85.1",
        "lib UPSTREAM_VERSION must match manifest pin"
    );
}

#[test]
fn gate_2_source_sha_is_present_and_full_length() {
    let m = manifest_text();
    let v = extract_after(&m, "\nsource_sha ").or_else(|| extract_after(&m, "\nsource_sha="));
    let sha = v.as_deref().expect("source_sha must be present");
    assert_eq!(
        sha.len(),
        40,
        "source_sha must be a full 40-char git commit hex (got {:?})",
        sha
    );
    assert!(
        sha.chars().all(|c| c.is_ascii_hexdigit()),
        "source_sha must be hex; got {:?}",
        sha
    );
    // Pinned commit for litellm v1.85.1
    assert!(
        sha.starts_with("f9c2a417"),
        "source_sha must start with f9c2a417 (litellm v1.85.1); got {:?}",
        sha
    );
}

#[test]
fn gate_3_fill_ratio_is_measured_and_at_least_floor() {
    let m = manifest_text();
    let raw = extract_after(&m, "\nfill_ratio ").or_else(|| extract_after(&m, "\nfill_ratio="));
    let ratio: f64 = raw
        .as_deref()
        .expect("[parity] fill_ratio must be present")
        .parse()
        .expect("fill_ratio must parse as float");
    assert!(
        ratio >= 0.95,
        "cave-llm-gateway parity floor: fill_ratio must be >= 0.95 (got {}). \
         Either improve coverage or document scope-cuts as [[skipped]].",
        ratio
    );
    assert!(ratio <= 1.0, "fill_ratio must be a fraction (got {})", ratio);
}

#[test]
fn gate_4_parity_ratio_source_is_manifest() {
    let m = manifest_text();
    let v = extract_after(&m, "\nparity_ratio_source ")
        .or_else(|| extract_after(&m, "\nparity_ratio_source="));
    assert_eq!(
        v.as_deref(),
        Some("manifest"),
        "parity_ratio_source must be \"manifest\" so the workspace parity-index \
         reads fill_ratio from this file rather than an external audit doc"
    );
}

#[test]
fn gate_5_last_audit_is_a_2026_iso_date() {
    let m = manifest_text();
    let when = extract_after(&m, "\nlast_audit ").or_else(|| extract_after(&m, "\nlast_audit="));
    let when = when.expect("[parity] last_audit must be present");
    // Relaxed from a hard 2026-05-21 pin: each honest-uplift continuation
    // re-audits and bumps the date, so assert a well-formed 2026 ISO date.
    assert!(
        when.starts_with("2026-") && when.len() == 10,
        "[parity] last_audit must be a 2026 ISO date (YYYY-MM-DD); got {:?}",
        when
    );
}

#[test]
fn gate_6_mapped_partial_skipped_unmapped_sum_to_total_with_floor() {
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
        "cave-llm-gateway MVP floor: >= 20 mapped subsystems (got {})",
        mapped
    );
    assert_eq!(
        unmapped, 0,
        "Charter v2 honest-fill: all subsystems must be classified (got {} unmapped)",
        unmapped
    );
}

#[test]
fn gate_7_no_stub_macros_in_src() {
    let root: PathBuf = [env!("CARGO_MANIFEST_DIR"), "src"].iter().collect();
    let mut offenders = Vec::new();
    walk(&root, &mut |p| {
        if p.extension().map(|e| e == "rs").unwrap_or(false)
            && let Ok(s) = fs::read_to_string(p)
        {
            for (i, line) in s.lines().enumerate() {
                let code = line.split("//").next().unwrap_or("");
                if code.contains("unimplemented!(") || code.contains("todo!(") {
                    offenders.push(format!("{}:{}", p.display(), i + 1));
                }
            }
        }
    });
    assert!(
        offenders.is_empty(),
        "Charter v2 no-stub gate failed: {:?}",
        offenders
    );
}

#[test]
fn gate_8_every_rs_file_carries_agpl_spdx() {
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
        total >= 18,
        "expected >= 18 .rs files in cave-llm-gateway; got {}",
        total
    );
}

#[test]
fn gate_9_all_six_mvp_providers_dispatch_from_registry() {
    use cave_llm_gateway::provider::{ProviderConfig, ProviderRegistry, ProviderType};
    let configs = vec![
        ProviderConfig {
            name: "openai".into(),
            provider_type: ProviderType::OpenAi,
            base_url: "https://api.openai.com".into(),
            api_key: Some("x".into()),
            timeout_secs: 5,
            max_retries: 0,
            weight: 1,
            enabled: true,
        },
        ProviderConfig {
            name: "anthropic".into(),
            provider_type: ProviderType::Anthropic,
            base_url: "https://api.anthropic.com".into(),
            api_key: Some("x".into()),
            timeout_secs: 5,
            max_retries: 0,
            weight: 1,
            enabled: true,
        },
        ProviderConfig {
            name: "ollama".into(),
            provider_type: ProviderType::Ollama,
            base_url: "http://127.0.0.1:11434".into(),
            api_key: None,
            timeout_secs: 5,
            max_retries: 0,
            weight: 1,
            enabled: true,
        },
        ProviderConfig {
            name: "llamacpp".into(),
            provider_type: ProviderType::LlamaCpp,
            base_url: "http://127.0.0.1:8080".into(),
            api_key: None,
            timeout_secs: 5,
            max_retries: 0,
            weight: 1,
            enabled: true,
        },
        ProviderConfig {
            name: "mlx".into(),
            provider_type: ProviderType::Mlx,
            base_url: "http://127.0.0.1:8081".into(),
            api_key: None,
            timeout_secs: 5,
            max_retries: 0,
            weight: 1,
            enabled: true,
        },
        ProviderConfig {
            name: "mistral".into(),
            provider_type: ProviderType::Mistral,
            base_url: "".into(),
            api_key: Some("x".into()),
            timeout_secs: 5,
            max_retries: 0,
            weight: 1,
            enabled: true,
        },
    ];
    let reg = ProviderRegistry::from_config(configs);
    for want in ["openai", "anthropic", "ollama", "llamacpp", "mlx", "mistral"] {
        assert!(
            reg.get(want).is_some(),
            "provider `{}` must dispatch from from_config",
            want
        );
    }
    // Capability router seed catalogue must also know every MVP provider.
    let cats: std::collections::HashSet<_> = cave_llm_gateway::seed_catalogue()
        .into_iter()
        .map(|c| c.provider)
        .collect();
    for want in ["openai", "anthropic", "ollama", "llamacpp", "mlx", "mistral"] {
        assert!(cats.contains(want), "capability seed missing {}", want);
    }
    // hermes bridge classifies all six.
    use cave_llm_gateway::classify_hermes_provider;
    for want in ["openai", "anthropic", "ollama", "llamacpp", "mlx", "mistral"] {
        assert!(
            classify_hermes_provider(want).is_some(),
            "hermes bridge can't classify {}",
            want
        );
    }
}

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

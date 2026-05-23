// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Smoke tests — five end-to-end scenarios that exercise the engine
//! against realistic input shapes. They confirm that the detector
//! registry, decoder pipeline, dedup gate, and output writers all line up.

use cave_trufflehog::config::ScanConfig;
use cave_trufflehog::custom_detectors::{compile, load_spec_yaml};
use cave_trufflehog::engine::Engine;
use cave_trufflehog::models::{Chunk, DetectorType, SourceKind, SourceMetadata};
use cave_trufflehog::output::OutputFormat;
use cave_trufflehog::sources::filesystem::FilesystemSource;
use cave_trufflehog::sources::git::GitSource;
use cave_trufflehog::sources::Source;
use std::fs;
use tempfile::TempDir;

fn mk_chunk(payload: &[u8]) -> Chunk {
    let mut c = Chunk::new("filesystem", "/repo/x.go", payload.to_vec());
    c.source_metadata = SourceMetadata {
        kind: SourceKind::Filesystem,
        file: Some("/repo/x.go".into()),
        ..Default::default()
    };
    c
}

#[test]
fn smoke_1_multi_provider_in_one_chunk() {
    let payload = format!(
        "aws_id=AKIAIOSFODNN7EXAMPLE secret=wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY12 \
         stripe=sk_live_1234567890abcdefghij \
         slack=xoxb-1111111111-2222222222-AbCdEf \
         github=ghp_1234567890ABCDEFGHIJKLMNOPQRSTUVWXYZab \
         anthropic=sk-ant-{} \
         openai=sk-{}",
        "a".repeat(60),
        "b".repeat(48),
    );
    let e = Engine::new(ScanConfig::default());
    let f = e.scan_chunk(&mk_chunk(payload.as_bytes()));
    let mut types: Vec<_> = f.iter().map(|x| x.result.detector_type).collect();
    types.sort_by_key(|t| *t as u32);
    types.dedup();
    assert!(types.contains(&DetectorType::Aws));
    assert!(types.contains(&DetectorType::Stripe));
    assert!(types.contains(&DetectorType::Slack));
    assert!(types.contains(&DetectorType::Github));
    assert!(types.contains(&DetectorType::Anthropic));
    assert!(types.contains(&DetectorType::Openai));
    assert!(types.len() >= 6);
}

#[test]
fn smoke_2_filesystem_source_pipeline() {
    let td = TempDir::new().unwrap();
    fs::write(
        td.path().join("config.env"),
        b"STRIPE_KEY=sk_live_zzzzzzzzzzzzzzzzzzzz",
    )
    .unwrap();
    fs::write(td.path().join("README.md"), b"nothing here").unwrap();
    let s = FilesystemSource::new(td.path());
    let chunks = s.chunks().unwrap();
    let e = Engine::new(ScanConfig::default());
    let mut all = Vec::new();
    for c in &chunks {
        all.extend(e.scan_chunk(c));
    }
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].result.detector_type, DetectorType::Stripe);
    assert!(all[0]
        .source_metadata
        .file
        .as_deref()
        .unwrap()
        .ends_with("config.env"));
}

#[test]
fn smoke_3_git_history_pipeline() {
    let td = TempDir::new().unwrap();
    let repo = git2::Repository::init(td.path()).unwrap();
    fs::write(td.path().join("a.txt"), b"key=ghp_1234567890ABCDEFGHIJKLMNOPQRSTUVWXYZab").unwrap();
    let mut idx = repo.index().unwrap();
    idx.add_path(std::path::Path::new("a.txt")).unwrap();
    idx.write().unwrap();
    let tree_oid = idx.write_tree().unwrap();
    let tree = repo.find_tree(tree_oid).unwrap();
    let sig = git2::Signature::now("smoke", "smoke@cave.dev").unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
        .unwrap();
    let s = GitSource::new(td.path());
    let e = Engine::new(ScanConfig::default());
    let mut all = Vec::new();
    for c in s.chunks().unwrap() {
        all.extend(e.scan_chunk(&c));
    }
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].result.detector_type, DetectorType::Github);
    assert!(all[0].source_metadata.commit.is_some());
}

#[test]
fn smoke_4_custom_detector_yaml() {
    let yaml = r#"
detectors:
  - name: AcmeInternal
    keywords: ["acme_"]
    regex:
      token: 'acme_[A-Z0-9]{20,}'
    min_entropy: 2.5
"#;
    let specs = load_spec_yaml(yaml).unwrap();
    let cd = compile(specs[0].clone()).unwrap();
    let r = cd.scan(b"unrelated noise acme_ABCDEFGHIJKL1234567890 trailing");
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].detector_type, DetectorType::Custom);
    assert_eq!(r[0].detector_name, "AcmeInternal");
}

#[test]
fn smoke_5_output_pipeline_all_four_formats() {
    let e = Engine::new(ScanConfig::default());
    let _f = e.scan_chunk(&mk_chunk(
        b"AKIAIOSFODNN7EXAMPLE wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY12",
    ));
    let findings = e.findings();
    assert!(!findings.is_empty());
    for fmt in [
        OutputFormat::Json,
        OutputFormat::Jsonl,
        OutputFormat::Plain,
        OutputFormat::GithubActions,
    ] {
        let mut buf = Vec::new();
        fmt.write(&mut buf, &findings).unwrap();
        assert!(!buf.is_empty(), "format {:?} produced empty output", fmt);
    }
}

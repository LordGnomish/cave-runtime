// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Charter v2 self-audit — cave-workflows must carry an honest, measured
//! `fill_ratio` against upstream argoproj/argo-workflows v4.0.5, a pinned
//! `source_sha`, the 2026-05-24 close-out audit date,
//! `parity_ratio_source = "manifest"`, 100% AGPL SPDX header coverage,
//! no stub macros in `src/`, mapped+partial+skipped+unmapped summing to
//! total, and the full Workflow / Template / executor public surface
//! reachable through `cave_workflows`.
//!
//! 9 assertions — one per gate of the close-out checklist.

use std::fs;
use std::path::PathBuf;

const TODAY: &str = "2026-05-28";
const FLOOR_FILL_RATIO: f64 = 0.95;
const PINNED_VERSION: &str = "v4.0.5";
const PINNED_SHA: &str = "0ab1452144d8f4d57c50b37ce50dad218868e950";

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

#[test]
fn assertion_1_upstream_version_pinned() {
    let m = manifest_text();
    let v = extract_after(&m, "\nversion ").or_else(|| extract_after(&m, "\nversion="));
    assert_eq!(v.as_deref(), Some(PINNED_VERSION));
}

#[test]
fn assertion_2_source_sha_matches_version() {
    let m = manifest_text();
    let sha = extract_after(&m, "\nsource_sha ").or_else(|| extract_after(&m, "\nsource_sha="));
    assert_eq!(sha.as_deref(), Some(PINNED_SHA));
}

#[test]
fn assertion_3_fill_ratio_meets_floor() {
    let m = manifest_text();
    let raw = extract_after(&m, "\nfill_ratio ")
        .or_else(|| extract_after(&m, "\nfill_ratio="))
        .expect("fill_ratio");
    let ratio: f64 = raw.parse().expect("fill_ratio must parse");
    assert!(
        ratio >= FLOOR_FILL_RATIO,
        "fill_ratio must be >= {} (got {})",
        FLOOR_FILL_RATIO,
        ratio
    );
    assert!(ratio <= 1.0);
}

#[test]
fn assertion_4_parity_ratio_source_is_manifest() {
    let m = manifest_text();
    let v = extract_after(&m, "\nparity_ratio_source ")
        .or_else(|| extract_after(&m, "\nparity_ratio_source="));
    assert_eq!(v.as_deref(), Some("manifest"));
}

#[test]
fn assertion_5_last_audit_is_today() {
    let m = manifest_text();
    let when = extract_after(&m, "\nlast_audit ").or_else(|| extract_after(&m, "\nlast_audit="));
    assert_eq!(when.as_deref(), Some(TODAY));
}

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
    assert_eq!(mapped + partial + skipped + unmapped, total);
    assert!(mapped >= 15, ">= 15 mapped (got {})", mapped);
}

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
    assert!(missing.is_empty(), "missing AGPL SPDX: {:?}", missing);
    assert!(total >= 6);
}

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
    assert!(offenders.is_empty(), "stub gate failed: {}", offenders.join("\n"));
}

#[test]
fn assertion_9_workflows_surface_intact() {
    use cave_workflows::executor::{
        aggregate_phase, next_actions, parse_duration_seconds, retry_decision,
    };
    use cave_workflows::store::WorkflowStore;
    use cave_workflows::workflow_crd::{
        topo_order, Arguments, ContainerTemplate, DagTask, DagTemplate, Inputs, Outputs,
        RetryStrategy, Template, TemplateBody, Workflow, WorkflowPhase, WorkflowSpec,
    };
    use std::collections::HashMap;

    // 1. Validation + DAG + Steps reach
    let wf = Workflow::new(
        "x",
        "argo",
        WorkflowSpec {
            entrypoint: "main".into(),
            templates: vec![Template {
                name: "main".into(),
                inputs: Inputs::default(),
                outputs: Outputs::default(),
                body: TemplateBody::Container(ContainerTemplate {
                    image: "alpine".into(),
                    command: vec![],
                    args: vec![],
                    env: HashMap::new(),
                    working_dir: None,
                }),
                retry_strategy: None,
                timeout: None,
            }],
            arguments: Arguments::default(),
            service_account_name: None,
            on_exit: None,
            parallelism: None,
            workflow_template_ref: None,
        },
    );
    assert!(wf.validate().is_ok());
    let actions = next_actions(&wf);
    assert!(!actions.is_empty());
    assert_eq!(aggregate_phase(&wf), WorkflowPhase::Pending);

    // 2. Toposort
    let order = topo_order(&[
        DagTask {
            name: "a".into(),
            template: "t".into(),
            dependencies: vec![],
            arguments: Arguments::default(),
            when: None,
        },
        DagTask {
            name: "b".into(),
            template: "t".into(),
            dependencies: vec!["a".into()],
            arguments: Arguments::default(),
            when: None,
        },
    ])
    .unwrap();
    assert_eq!(order, vec!["a".to_string(), "b".to_string()]);

    // 3. Retry decision + duration parser
    let s = RetryStrategy {
        limit: 3,
        retry_policy: "OnFailure".into(),
        backoff: None,
    };
    assert_eq!(retry_decision(Some(&s), 1, WorkflowPhase::Failed), Some(2));
    assert_eq!(parse_duration_seconds("5m"), Some(300));

    // 4. Store
    let store = WorkflowStore::new();
    store.create(wf.clone()).unwrap();
    assert_eq!(store.list(Some("argo")).len(), 1);

    // 5. Empty DAG dag template constructs
    let _ = TemplateBody::Dag(DagTemplate {
        tasks: vec![],
        fail_fast: Some(true),
    });
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

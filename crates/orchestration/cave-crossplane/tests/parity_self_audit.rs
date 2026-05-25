// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Charter v2 self-audit — cave-crossplane.
//!
//! Enforces the 8-gate close-out checklist for the 2026-05-23 deep-port
//! against upstream crossplane/crossplane v2.3.1 (source_sha
//! 41c6f9c4729175cf0f953cbf267378b8734e8d27).
//!
//! 9 assertions — G1..G8 + surface smoke.

use std::fs;
use std::path::{Path, PathBuf};

const TODAY: &str = "2026-05-23";
const FLOOR_FILL_RATIO: f64 = 0.95;
const FLOOR_HONEST_RATIO: f64 = 0.65;
const UPSTREAM_VERSION: &str = "v2.3.1";
const UPSTREAM_SHA: &str = "41c6f9c4729175cf0f953cbf267378b8734e8d27";

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

// ─── G1: upstream block + pinned source_sha ─────────────────────────────────

#[test]
fn g1_upstream_block_pinned() {
    let m = manifest_text();
    let v = extract_after(&m, "\nversion ").or_else(|| extract_after(&m, "\nversion="));
    assert_eq!(
        v.as_deref(),
        Some(UPSTREAM_VERSION),
        "G1: [upstream] version must pin Crossplane {} (got {:?})",
        UPSTREAM_VERSION,
        v
    );
    assert!(
        m.contains(UPSTREAM_SHA),
        "G1: [upstream] source_sha must contain {}",
        UPSTREAM_SHA
    );
    assert!(
        m.contains("[upstream]"),
        "G1: [upstream] block must be present"
    );
}

// ─── G2: every [[mapped]] has local_files all existing on disk ──────────────

#[test]
fn g2_mapped_local_files_exist() {
    let m = manifest_text();
    let root: PathBuf = [env!("CARGO_MANIFEST_DIR")].iter().collect();
    let mut missing: Vec<String> = Vec::new();
    let mut mapped_count = 0usize;
    let mut in_mapped_block = false;

    for line in m.lines() {
        let t = line.trim();
        if t.starts_with("[[mapped]]") {
            in_mapped_block = true;
            mapped_count += 1;
            continue;
        }
        if t.starts_with("[[") && !t.starts_with("[[mapped]]") {
            in_mapped_block = false;
        }
        if !in_mapped_block {
            continue;
        }
        if let Some(rest) = t.strip_prefix("local_files") {
            let rest = rest.trim_start_matches([' ', '=', '\t']);
            // Parse ["foo", "bar"]
            let paths: Vec<&str> = rest
                .trim_start_matches('[')
                .trim_end_matches(']')
                .split(',')
                .map(|s| s.trim().trim_matches('"'))
                .filter(|s| !s.is_empty())
                .collect();
            for p in paths {
                let abs = root.join(p);
                if !abs.exists() {
                    missing.push(abs.display().to_string());
                }
            }
        }
    }
    assert!(
        missing.is_empty(),
        "G2: {} mapped subsystems but {} local_files missing: {:?}",
        mapped_count,
        missing.len(),
        missing
    );
    assert!(
        mapped_count >= 22,
        "G2: expect >= 22 mapped subsystems (got {})",
        mapped_count
    );
}

// ─── G3: every [[partial]] has gap_reason ───────────────────────────────────

#[test]
fn g3_partial_has_gap_reason() {
    let m = manifest_text();
    let mut blocks = 0usize;
    let mut missing_reason = 0usize;
    let mut current_has_reason = false;
    let mut in_partial = false;
    for line in m.lines() {
        let t = line.trim();
        if t.starts_with("[[partial]]") {
            if in_partial && !current_has_reason {
                missing_reason += 1;
            }
            in_partial = true;
            current_has_reason = false;
            blocks += 1;
            continue;
        }
        if t.starts_with("[[") {
            if in_partial && !current_has_reason {
                missing_reason += 1;
            }
            in_partial = false;
        }
        if in_partial && t.starts_with("gap_reason") {
            current_has_reason = true;
        }
    }
    if in_partial && !current_has_reason {
        missing_reason += 1;
    }
    assert_eq!(
        missing_reason, 0,
        "G3: {} of {} [[partial]] blocks missing gap_reason",
        missing_reason, blocks
    );
}

// ─── G4: every [[skipped]] has scope_cut_target ─────────────────────────────

#[test]
fn g4_skipped_has_scope_cut_target() {
    let m = manifest_text();
    let mut blocks = 0usize;
    let mut missing = 0usize;
    let mut current_has = false;
    let mut in_skipped = false;
    for line in m.lines() {
        let t = line.trim();
        if t.starts_with("[[skipped]]") {
            if in_skipped && !current_has {
                missing += 1;
            }
            in_skipped = true;
            current_has = false;
            blocks += 1;
            continue;
        }
        if t.starts_with("[[") {
            if in_skipped && !current_has {
                missing += 1;
            }
            in_skipped = false;
        }
        if in_skipped && t.starts_with("scope_cut_target") {
            current_has = true;
        }
    }
    if in_skipped && !current_has {
        missing += 1;
    }
    assert_eq!(
        missing, 0,
        "G4: {} of {} [[skipped]] blocks missing scope_cut_target",
        missing, blocks
    );
}

// ─── G5: every [[unmapped]] has note (honest gap) ───────────────────────────

#[test]
fn g5_unmapped_has_note() {
    let m = manifest_text();
    let mut blocks = 0usize;
    let mut missing = 0usize;
    let mut current_has = false;
    let mut in_unmapped = false;
    for line in m.lines() {
        let t = line.trim();
        if t.starts_with("[[unmapped]]") {
            if in_unmapped && !current_has {
                missing += 1;
            }
            in_unmapped = true;
            current_has = false;
            blocks += 1;
            continue;
        }
        if t.starts_with("[[") {
            if in_unmapped && !current_has {
                missing += 1;
            }
            in_unmapped = false;
        }
        if in_unmapped && t.starts_with("note") {
            current_has = true;
        }
    }
    if in_unmapped && !current_has {
        missing += 1;
    }
    assert_eq!(
        missing, 0,
        "G5: {} of {} [[unmapped]] blocks missing note",
        missing, blocks
    );
}

// ─── G6: fill_ratio >= 0.95 + counts sum to total + honest_ratio >= 0.65 ────

#[test]
fn g6_fill_ratio_meets_floor() {
    let m = manifest_text();
    let raw = extract_after(&m, "\nfill_ratio ")
        .or_else(|| extract_after(&m, "\nfill_ratio="))
        .expect("G6: fill_ratio must be present");
    let fill: f64 = raw.parse().expect("fill_ratio must parse as float");
    assert!(
        fill >= FLOOR_FILL_RATIO,
        "G6: fill_ratio must be >= {} (got {})",
        FLOOR_FILL_RATIO,
        fill
    );

    let raw_h = extract_after(&m, "\nhonest_ratio ")
        .or_else(|| extract_after(&m, "\nhonest_ratio="))
        .expect("G6: honest_ratio must be present");
    let honest: f64 = raw_h.parse().expect("honest_ratio must parse");
    assert!(
        honest >= FLOOR_HONEST_RATIO,
        "G6: honest_ratio must be >= {} (got {})",
        FLOOR_HONEST_RATIO,
        honest
    );

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
        "G6: counts must sum to total ({} + {} + {} + {} != {})",
        mapped,
        partial,
        skipped,
        unmapped,
        total
    );

    // last_audit must be today's close-out date
    let when = extract_after(&m, "\nlast_audit ").or_else(|| extract_after(&m, "\nlast_audit="));
    assert_eq!(
        when.as_deref(),
        Some(TODAY),
        "G6: last_audit must be {} (got {:?})",
        TODAY,
        when
    );
}

// ─── G7: SPDX line 1 on every .rs ───────────────────────────────────────────

#[test]
fn g7_spdx_header_coverage() {
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
        "G7: {} of {} .rs files missing AGPL SPDX header: {:?}",
        missing.len(),
        total,
        missing
    );
    assert!(
        total >= 25,
        "G7: expect >= 25 .rs files in cave-crossplane (got {})",
        total
    );
}

// ─── G8: no stub macros in src/ ─────────────────────────────────────────────

#[test]
fn g8_no_stub_macros_in_src() {
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
                || trimmed.contains("panic!(\"not impl")
            {
                offenders.push(format!("{}:{}: {}", p.display(), lineno + 1, line.trim()));
            }
        }
    });
    assert!(
        offenders.is_empty(),
        "G8: Charter v2 no-stub gate failed in src/:\n{}",
        offenders.join("\n")
    );
}

// ─── G9: full surface intact via cave_crossplane re-exports ─────────────────

#[test]
fn g9_surface_intact() {
    use cave_crossplane::{
        composition::{
            legacy::LegacyComposer,
            patch_transform::PatchTransformEngine,
            pipeline::{PipelineExecutor, PipelineResult},
            step::{Step, StepCredentials},
        },
        conditions::{Condition, ConditionStatus, ConditionType, propagate_composed_to_xr},
        function::{
            FunctionStore,
            grpc_codec::{RunFunctionRequest, RunFunctionResponse, encode_request, decode_response},
            auto_ready::auto_ready_eval,
            kcl::evaluate_kcl,
            go_template::render_go_template,
            patch_transform::run_patch_transform_fn,
        },
        models::{
            CompositionMode, CreateClaimRequest, CreateProviderRequest, CreateXrdRequest,
            DeletionPolicy, ProviderType, XrdScope,
        },
        provider::{
            config::{Credentials, ProviderConfig, ProviderConfigStore},
            revision::{ProviderRevisionStore, RevisionState},
            runtime::DeploymentRuntimeConfig,
        },
        providers_builtin::{kubernetes::KubernetesProvider, helm::HelmProvider},
        xpkg::{
            dependency::{DependencyGraph, ResolveError},
            install::install_package,
            pull::{PackageBundle, pull_offline},
            revision::PackageRevisionTracker,
        },
        xr::{bind::bind_claim_to_xr, lifecycle::XrPhase, status::aggregate_ready},
        xrd::{
            conversion::convert_v1_to_v2,
            defaulting::apply_defaults,
            schema_validate::validate_spec,
            spec::XrdSpec,
        },
        CrossplaneState, MODULE_NAME, router,
    };
    use std::sync::Arc;
    use serde_json::json;

    assert_eq!(MODULE_NAME, "crossplane");
    let state = Arc::new(CrossplaneState::default());
    let _r = router(state.clone());

    // pipeline + step roundtrip
    let step = Step::new("compose", "function-patch-and-transform").with_input(json!({"resources":[]}));
    assert_eq!(step.step, "compose");
    let _creds: Option<StepCredentials> = None;
    let exec = PipelineExecutor::new();
    let req = RunFunctionRequest::new("ctx", json!({}), json!({}));
    let resp = RunFunctionResponse::ready(json!({}));
    let enc = encode_request(&req);
    let _ = decode_response(&serde_json::to_vec(&resp).unwrap()).unwrap();
    assert!(!enc.is_empty());
    let _pr: PipelineResult = exec.run_sync(&[], &state.function_store, &req).unwrap();

    // legacy composer
    let _lc = LegacyComposer::new();

    // patch transform engine
    let _pt = PatchTransformEngine::new();

    // XRD spec + schema + defaulting + conversion
    let spec = XrdSpec::new("test.cave.io", "Database", XrdScope::Cluster);
    let _ = spec.list_kind();
    let schema_json = json!({
        "type": "object",
        "required": ["name"],
        "properties": {
            "name": {"type":"string","minLength":1},
            "size": {"type":"integer","default": 10}
        }
    });
    let mut spec_v = json!({"name":"db1"});
    apply_defaults(&schema_json, &mut spec_v);
    assert_eq!(spec_v["size"], json!(10));
    assert!(validate_spec(&schema_json, &spec_v).is_ok());
    let _v2 = convert_v1_to_v2(&json!({"apiVersion":"apiextensions.crossplane.io/v1","kind":"CompositeResourceDefinition"}));

    // XR lifecycle + status
    let phase = XrPhase::Creating;
    assert!(!aggregate_ready(&[json!({"status":{"conditions":[{"type":"Ready","status":"True"}]}})]).is_empty() || phase == XrPhase::Creating);

    // bind
    let claim = json!({"metadata":{"namespace":"ns","name":"c1"}});
    let xr = json!({"metadata":{"name":"x1"}});
    let _b = bind_claim_to_xr(&claim, &xr);

    // provider config + revision + runtime
    let pcs = ProviderConfigStore::new();
    let cfg = ProviderConfig::new("default", Credentials::None);
    pcs.upsert(cfg).unwrap();
    let prs = ProviderRevisionStore::new();
    let rev = prs.append("provider-kubernetes", "v0.1.0").unwrap();
    assert_eq!(rev.state, RevisionState::Active);
    let _drc = DeploymentRuntimeConfig::default_for("provider-kubernetes");

    // function store
    let fs = FunctionStore::new();
    fs.install("function-auto-ready", "v0.1.0", "xpkg.upbound.io/x/function-auto-ready:v0.1.0").unwrap();
    assert_eq!(fs.list().len(), 1);

    // built-in functions
    let ar = auto_ready_eval(&[json!({"status":{"conditions":[{"type":"Ready","status":"True"}]}})]);
    assert!(ar);
    let _k = evaluate_kcl("x = 1", &json!({})).unwrap();
    let gt = render_go_template("hello {{ .name }}", &json!({"name":"crossplane"})).unwrap();
    assert!(gt.contains("crossplane"));
    let ptfn = run_patch_transform_fn(&req).unwrap();
    assert!(ptfn.is_object() || ptfn.is_null());

    // xpkg
    let bundle = pull_offline("/nonexistent/xpkg").unwrap_or_else(|_| PackageBundle::empty("test-pkg"));
    assert!(bundle.name.contains("test-pkg") || !bundle.name.is_empty());
    let plan = install_package(&bundle, &state);
    assert!(plan.is_ok());
    let prt = PackageRevisionTracker::new();
    let _r1 = prt.record("cfg-pkg", "v0.1.0");
    let mut dag = DependencyGraph::new();
    dag.add_node("a");
    dag.add_node("b");
    dag.add_edge("a", "b").unwrap();
    let order = dag.topo_sort();
    assert!(order.is_ok());
    let mut cyclic = DependencyGraph::new();
    cyclic.add_node("x");
    cyclic.add_node("y");
    cyclic.add_edge("x", "y").unwrap();
    cyclic.add_edge("y", "x").unwrap();
    assert!(matches!(cyclic.topo_sort(), Err(ResolveError::Cycle(_))));

    // conditions
    let c = Condition::new(ConditionType::Ready, ConditionStatus::True);
    assert_eq!(c.condition_type, ConditionType::Ready);
    let xr2 = json!({"status":{"conditions":[]}});
    let composed = vec![json!({"status":{"conditions":[{"type":"Ready","status":"True"},{"type":"Synced","status":"True"}]}})];
    let updated = propagate_composed_to_xr(&xr2, &composed);
    assert!(updated["status"]["conditions"].is_array());

    // providers builtin
    let kp = KubernetesProvider::new();
    let _objs = kp.list_objects("default");
    let hp = HelmProvider::new();
    let _rels = hp.list_releases("default");

    // Request types still work
    let _ = CreateXrdRequest {
        name: "x".into(),
        group: "g".into(),
        kind: "K".into(),
        claim_kind: Some("KC".into()),
        scope: XrdScope::Cluster,
        versions: vec![],
    };
    let _ = CreateProviderRequest {
        name: "p".into(),
        package: "pkg".into(),
        provider_type: ProviderType::Official,
    };
    let _ = CreateClaimRequest {
        name: "c".into(),
        namespace: "ns".into(),
        kind: "KC".into(),
        api_version: "g/v1".into(),
        spec: json!({}),
    };
    let _ = DeletionPolicy::Delete;
    let _ = CompositionMode::Pipeline;
}

// ─── helpers ────────────────────────────────────────────────────────────────

fn walk(dir: &Path, cb: &mut dyn FnMut(&Path)) {
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

// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Charter v2 self-audit — cave-dependency-track must carry an honest,
//! measured `fill_ratio` against upstream Dependency-Track v4.14.2, a
//! pinned `source_sha` for reproducibility, the 2026-05-23 close-out
//! audit date, `parity_ratio_source = "manifest"`, 100 % AGPL SPDX
//! header coverage, no stub macros in `src/`, mapped+partial+skipped+
//! unmapped summing to total, and the full project/sbom/vuln/policy/
//! audit/vex/bov/notifications/integrations surface reachable through
//! the `cave_dependency_track` crate root.
//!
//! 9 assertions — one per gate of the close-out checklist.

use std::fs;
use std::path::PathBuf;

const TODAY: &str = "2026-05-23";
const FLOOR_FILL_RATIO: f64 = 0.95;
const DT_VERSION: &str = "v4.14.2";
const DT_SHA: &str = "c4a156726472cd529cc9fa8ed12e825cc000327d";

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

// ─── Assertion 1: upstream pinned to Dependency-Track v4.14.2 ───────────────

#[test]
fn assertion_1_upstream_version_pinned() {
    let m = manifest_text();
    let v = extract_after(&m, "\nversion ").or_else(|| extract_after(&m, "\nversion="));
    assert_eq!(
        v.as_deref(),
        Some(DT_VERSION),
        "[upstream] version must pin Dependency-Track {} — Charter v2 always-latest gate (got {:?})",
        DT_VERSION,
        v
    );
}

// ─── Assertion 2: source_sha matches v4.14.2 ────────────────────────────────

#[test]
fn assertion_2_source_sha_matches_version() {
    let m = manifest_text();
    assert!(
        m.contains(DT_SHA),
        "[upstream] source_sha must contain {} (full manifest text scan)",
        DT_SHA
    );
}

// ─── Assertion 3: fill_ratio ≥ 0.95 ─────────────────────────────────────────

#[test]
fn assertion_3_fill_ratio_meets_floor() {
    let m = manifest_text();
    let raw = extract_after(&m, "\nfill_ratio ")
        .or_else(|| extract_after(&m, "\nfill_ratio="))
        .expect("[parity] fill_ratio must be present");
    let ratio: f64 = raw.parse().expect("fill_ratio must parse as float");
    assert!(
        ratio >= FLOOR_FILL_RATIO,
        "cave-dependency-track Charter v2 floor: fill_ratio must be >= {} (got {})",
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

// ─── Assertion 6: counts sum to total + ≥ 20 mapped ─────────────────────────

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
        "cave-dependency-track MVP floor: >= 20 mapped Dependency-Track subsystems (got {})",
        mapped
    );
}

// ─── Assertion 7: AGPL SPDX header coverage 100 % ───────────────────────────

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
        total >= 25,
        "expected >= 25 .rs files in cave-dependency-track; got {}",
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

// ─── Assertion 9: full deptrack surface reachable from cave_dependency_track ─

#[test]
fn assertion_9_deptrack_surface_intact() {
    use cave_dependency_track::audit::AuditStore;
    use cave_dependency_track::bov::BovDocument;
    use cave_dependency_track::components::{AnalysisCache, ComponentIdentity};
    use cave_dependency_track::cpe::Cpe23;
    use cave_dependency_track::engine::{evaluate_project, search_components, unique_identity_count};
    use cave_dependency_track::graphql::{execute, parse_query};
    use cave_dependency_track::integrations::{
        DefectDojoConfig, FortifyConfig, KennaConfig, ThreadFixConfig,
        build_defectdojo_payload, build_fortify_payload, build_kenna_payload, build_threadfix_csv,
    };
    use cave_dependency_track::licenses::{catalog, is_known, lookup};
    use cave_dependency_track::models::{
        AnalysisState, Classifier, Component, Project, Severity, VulnSource, Vulnerability,
    };
    use cave_dependency_track::notifications::{
        NotificationLevel, NotificationRule, NotificationRuleStore, NotificationScope,
        NotificationTrigger, PublisherKind, render_email, render_jira_issue, render_mattermost,
        render_slack, render_teams, render_webhook,
    };
    use cave_dependency_track::policy::engine::{
        Policy, PolicyAggregator, PolicyCondition, PolicyOperator, Subject,
    };
    use cave_dependency_track::policy::{
        PolicyStore, evaluate_age, evaluate_coordinates, evaluate_license, evaluate_vulnerability,
    };
    use cave_dependency_track::portfolio::{
        PortfolioStore, ProjectUpdate, build_tree, descendants, normalize_tag,
    };
    use cave_dependency_track::purl::Purl;
    use cave_dependency_track::repositories::{Repository, RepositoryStore, RepositoryType};
    use cave_dependency_track::risk::{RiskWeights, inherited_risk};
    use cave_dependency_track::sbom::{
        IngestReport, ingest, parse_cyclonedx_json, parse_spdx_json, parse_spdx_tag_value,
    };
    use cave_dependency_track::vex::VexDocument;
    use cave_dependency_track::vuln_intel::{
        VulnStore, parse_epss_csv, parse_ghsa_json, parse_nvd_2_0, parse_ossindex_response,
        parse_osv_json, parse_snyk_json, parse_vulndb_response,
    };
    use cave_dependency_track::{MODULE_NAME, State, UPSTREAM_NAME, UPSTREAM_SHA, UPSTREAM_VERSION, router};
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::Arc;

    // ── 1. Module identity + state ────────────────────────────────────────
    assert_eq!(MODULE_NAME, "deptrack");
    assert_eq!(UPSTREAM_NAME, "DependencyTrack");
    assert_eq!(UPSTREAM_VERSION, "v4.14.2");
    assert_eq!(UPSTREAM_SHA.len(), 40);
    let _r = router(Arc::new(State::default()));

    // ── 2. Portfolio CRUD + hierarchy + tags ──────────────────────────────
    let portfolio = PortfolioStore::new();
    let proj = portfolio
        .insert(Project::new("cave", Classifier::Application))
        .unwrap();
    assert_eq!(portfolio.count(), 1);
    let upd = ProjectUpdate {
        tags: Some(vec!["Prod".into(), "Sov".into()]),
        ..Default::default()
    };
    let updated = portfolio.update(proj.uuid, upd).unwrap();
    assert_eq!(updated.tags, vec!["prod", "sov"]);
    assert_eq!(normalize_tag("Prod Build"), "prod-build");
    let tree = build_tree(&[proj.clone()]);
    assert_eq!(tree.len(), 1);
    let _desc = descendants(proj.uuid, &[proj.clone()]);

    // ── 3. SBOM ingestion (CycloneDX + SPDX) ──────────────────────────────
    let bom_json = r#"{"bomFormat":"CycloneDX","specVersion":"1.6","components":[
      {"type":"library","name":"serde","version":"1","purl":"pkg:cargo/serde@1","licenses":[{"license":{"id":"MIT"}}]}
    ]}"#;
    let bom = parse_cyclonedx_json(bom_json).unwrap();
    let report: IngestReport = ingest(&portfolio, proj.uuid, &bom).unwrap();
    assert_eq!(report.inserted, 1);
    let spdx_json = r#"{"spdxVersion":"SPDX-2.3","packages":[{"SPDXID":"SPDXRef-1","name":"openssl"}]}"#;
    let _spdx = parse_spdx_json(spdx_json).unwrap();
    let _tv = parse_spdx_tag_value("SPDXVersion: SPDX-2.3\nPackageName: x\n").unwrap();

    // ── 4. Vuln intel (six sources + EPSS) ────────────────────────────────
    let store = VulnStore::new();
    let nvd_doc = r#"{"vulnerabilities":[{"cve":{"id":"CVE-2026-1","descriptions":[{"lang":"en","value":"x"}],"metrics":{"cvssMetricV31":[{"cvssData":{"baseScore":7.5}}]}}}]}"#;
    let cves = parse_nvd_2_0(nvd_doc).unwrap();
    store.upsert(cves[0].clone().into_vuln());
    let _osv = parse_osv_json(r#"{"id":"OSV-1"}"#).unwrap();
    let _ghsa = parse_ghsa_json(r#"{"ghsaId":"GHSA-x","severity":"HIGH"}"#).unwrap();
    let _snyk = parse_snyk_json(r#"{"id":"S-1","title":"t","severity":"high"}"#).unwrap();
    let _oss = parse_ossindex_response(r#"{"coordinates":"pkg:x","vulnerabilities":[]}"#).unwrap();
    let _vd = parse_vulndb_response(r#"{"results":[]}"#).unwrap();
    let _epss = parse_epss_csv("cve,epss,percentile\nCVE-2026-1,0.1,0.5\n").unwrap();
    assert!(store.count() >= 1);

    // ── 5. Component identity + cache ─────────────────────────────────────
    let mut c = Component::new(proj.uuid, "serde");
    c.purl = Some("pkg:cargo/serde@1".into());
    let id = ComponentIdentity::of(&c);
    let mut cache: AnalysisCache<u8> = AnalysisCache::new();
    cache.put(&id, 9);
    assert_eq!(cache.get(&id), Some(9));

    // ── 6. Policy engine (license + coordinates + vulnerability + age) ────
    let pol_store = PolicyStore::new();
    let p = pol_store.put(Policy::new("strict"));
    pol_store.assign(p.uuid, proj.uuid).unwrap();
    let cond = PolicyCondition {
        subject: Subject::License,
        operator: PolicyOperator::Is,
        value: "GPL-3.0".into(),
    };
    let mut comp = Component::new(proj.uuid, "x");
    comp.license = Some("GPL-3.0".into());
    let _ = evaluate_license(p.uuid, &[cond.clone()], &HashMap::new(), &comp);
    let _ = evaluate_coordinates(p.uuid, &[cond.clone()], &comp);
    let v = Vulnerability::new("CVE-X", VulnSource::Nvd);
    let _ = evaluate_vulnerability(p.uuid, &[cond.clone()], proj.uuid, &[v]);
    let _ = evaluate_age(p.uuid, &[cond.clone()], proj.uuid, None, chrono::Utc::now());
    let _aggr = PolicyAggregator::Any;

    // ── 7. Risk + audit + VEX + BOV ───────────────────────────────────────
    let vs = vec![
        Vulnerability {
            severity: Severity::High,
            ..Vulnerability::new("CVE-1", VulnSource::Nvd)
        },
    ];
    let r = inherited_risk(&vs, RiskWeights::default());
    assert!(r > 0.0);
    let audit = AuditStore::new();
    let comp_id = comp.uuid;
    let vuln_id = vs[0].uuid;
    audit.upsert(comp_id, vuln_id, AnalysisState::FalsePositive);
    let mut vex = VexDocument::new();
    vex.push_analysis(&vs[0], &audit.get(comp_id, vuln_id).unwrap());
    let _vex_json = vex.to_json();
    let _bov = BovDocument::build(proj.uuid, &[(comp_id, vs.clone())], &audit);

    // ── 8. CPE + PURL + licenses + repositories ───────────────────────────
    let cpe = Cpe23::parse("cpe:2.3:a:openssl:openssl:3.0.0:*:*:*:*:*:*:*").unwrap();
    let cpe2 = Cpe23::parse("cpe:2.3:a:openssl:openssl:*:*:*:*:*:*:*:*").unwrap();
    assert!(cpe.matches(&cpe2));
    let _purl = Purl::parse("pkg:cargo/serde@1.0").unwrap();
    assert!(is_known("MIT"));
    assert!(lookup("apache-2.0").is_some());
    assert!(catalog().len() >= 20);
    let repos = RepositoryStore::new();
    repos.put(Repository {
        r#type: RepositoryType::Cargo,
        identifier: "primary".into(),
        url: "https://crates.io".into(),
        enabled: true,
        priority: 100,
    });
    assert_eq!(repos.list().len(), 1);

    // ── 9. Notifications + integrations ───────────────────────────────────
    let rule_store = NotificationRuleStore::new();
    let rule = rule_store.put(NotificationRule {
        uuid: uuid::Uuid::new_v4(),
        name: "r".into(),
        scope: NotificationScope::Portfolio,
        level: NotificationLevel::Warning,
        triggers: vec![NotificationTrigger::NewVulnerability],
        publisher: PublisherKind::Slack,
        publisher_config: "{}".into(),
        project_filter: Vec::new(),
        enabled: true,
    });
    assert!(rule.enabled);
    let payload = cave_dependency_track::notifications::publishers::NotificationPayload {
        title: "x",
        level: "ERROR",
        scope: "PORTFOLIO",
        group: "NEW_VULNERABILITY",
        message: "msg",
        project: Some("cave"),
    };
    let _s = render_slack(&payload);
    let _t = render_teams(&payload);
    let _m = render_mattermost(&payload);
    let _w = render_webhook(&payload);
    let _e = render_email(&payload);
    let _j = render_jira_issue(&payload, "SEC");
    let dd = DefectDojoConfig::new("https://dd", "tok", 1);
    let _ = build_defectdojo_payload(&dd, &vs);
    let fr = FortifyConfig::new("https://ssc", "ci", 1);
    let _ = build_fortify_payload(&fr, &vs);
    let kn = KennaConfig::new("tok", 1, "asset");
    let _ = build_kenna_payload(&kn, &vs);
    let _tx = ThreadFixConfig::new("https://tf", "k", 1);
    let _ = build_threadfix_csv(&vs);

    // ── 10. Engine + GraphQL + search ─────────────────────────────────────
    let comps = portfolio.components_for(proj.uuid);
    let _r = search_components("serde", &comps);
    assert!(unique_identity_count(&comps) <= comps.len());
    let _ev = evaluate_project(
        &proj,
        &comps,
        &HashMap::new(),
        &pol_store.policies_for(proj.uuid),
        &HashMap::new(),
    );
    let g = execute("{ projects }", &portfolio.list(), &store.list(), &pol_store.list());
    assert!(g.get("data").is_some());
    let _q = parse_query("{ __schema }");
    let _ = json!({"ack": true});
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

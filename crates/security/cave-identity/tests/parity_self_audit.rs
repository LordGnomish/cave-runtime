// SPDX-License-Identifier: AGPL-3.0-or-later
//! Charter v2 self-audit — cave-identity must carry an honest, measured
//! `fill_ratio` against upstream spiffe/spire v1.15.0, a pinned
//! `source_sha`, the 2026-05-24 close-out audit date,
//! `parity_ratio_source = "manifest"`, 100% AGPL SPDX header coverage,
//! no stub macros in `src/`, mapped+partial+skipped+unmapped summing to
//! total, and the full identity public surface reachable through
//! `cave_identity`.
//!
//! 9 assertions — one per Charter v2 gate.

use std::fs;
use std::path::{Path, PathBuf};

const TODAY: &str = "2026-05-24";
const FLOOR_FILL_RATIO: f64 = 0.95;
const PINNED_VERSION: &str = "v1.15.0";
const PINNED_SHA: &str = "b7db9650aa98598ee7af21d7a75fbab8f6b70d42";

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

fn walk(dir: &Path, cb: &mut dyn FnMut(&Path)) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // Skip target/ which can balloon the walk.
            if path.file_name().and_then(|n| n.to_str()) == Some("target") {
                continue;
            }
            walk(&path, cb);
        } else {
            cb(&path);
        }
    }
}

// ─── G1: upstream version pinned ───────────────────────────────────────────

#[test]
fn gate_1_upstream_version_pinned() {
    let m = manifest_text();
    let v = extract_after(&m, "\nversion ").or_else(|| extract_after(&m, "\nversion="));
    assert_eq!(
        v.as_deref(),
        Some(PINNED_VERSION),
        "[upstream] version must pin SPIRE {} (got {:?})",
        PINNED_VERSION,
        v
    );
}

// ─── G2: source_sha matches commit for v1.15.0 ─────────────────────────────

#[test]
fn gate_2_source_sha_pinned() {
    let m = manifest_text();
    let sha = extract_after(&m, "\nsource_sha ").or_else(|| extract_after(&m, "\nsource_sha="));
    assert_eq!(
        sha.as_deref(),
        Some(PINNED_SHA),
        "source_sha must match the v1.15.0 tag commit (got {:?})",
        sha
    );
}

// ─── G3: fill_ratio >= 0.95 ────────────────────────────────────────────────

#[test]
fn gate_3_fill_ratio_meets_floor() {
    let m = manifest_text();
    let raw = extract_after(&m, "\nfill_ratio ")
        .or_else(|| extract_after(&m, "\nfill_ratio="))
        .expect("[parity] fill_ratio must be present");
    let ratio: f64 = raw.parse().expect("fill_ratio must parse as float");
    assert!(
        ratio >= FLOOR_FILL_RATIO,
        "cave-identity MVP floor: fill_ratio must be >= {} (got {})",
        FLOOR_FILL_RATIO,
        ratio
    );
    assert!(ratio <= 1.0, "fill_ratio must be a fraction (got {})", ratio);
}

// ─── G4: parity_ratio_source = "manifest" + last_audit = today ─────────────

#[test]
fn gate_4_parity_ratio_source_is_manifest() {
    let m = manifest_text();
    let src = extract_after(&m, "\nparity_ratio_source ")
        .or_else(|| extract_after(&m, "\nparity_ratio_source="));
    assert_eq!(src.as_deref(), Some("manifest"));
    let when = extract_after(&m, "\nlast_audit ").or_else(|| extract_after(&m, "\nlast_audit="));
    assert_eq!(
        when.as_deref(),
        Some(TODAY),
        "[parity] last_audit must reflect the {} Charter v2 close-out (got {:?})",
        TODAY,
        when
    );
}

// ─── G5: counts sum to total + >= 30 mapped ────────────────────────────────

#[test]
fn gate_5_counts_sum_to_total_and_30_mapped() {
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
        mapped >= 30,
        "cave-identity MVP floor: >= 30 mapped SPIRE subsystems (got {})",
        mapped
    );
    // Also verify the manifest's recorded fill_ratio agrees with the counts.
    let computed = (mapped + partial + skipped) as f64 / total as f64;
    let manifest_ratio: f64 = extract_after(&m, "\nfill_ratio ")
        .unwrap_or_default()
        .parse()
        .unwrap_or(0.0);
    assert!(
        (computed - manifest_ratio).abs() < 0.001,
        "fill_ratio {} disagrees with computed {}",
        manifest_ratio,
        computed
    );
}

// ─── G6: AGPL SPDX header coverage 100% ────────────────────────────────────

#[test]
fn gate_6_agpl_spdx_header_coverage() {
    let root: PathBuf = [env!("CARGO_MANIFEST_DIR")].iter().collect();
    let mut missing = Vec::new();
    let mut total = 0usize;
    walk(&root, &mut |p| {
        if p.extension().and_then(|e| e.to_str()) == Some("rs") {
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
        total >= 14,
        "expected >= 14 .rs files in cave-identity; got {}",
        total
    );
}

// ─── G7: no stub macros in src/ ────────────────────────────────────────────

#[test]
fn gate_7_no_stub_macros_in_src() {
    let src: PathBuf = [env!("CARGO_MANIFEST_DIR"), "src"].iter().collect();
    let mut offenders: Vec<String> = Vec::new();
    walk(&src, &mut |p| {
        if p.extension().and_then(|e| e.to_str()) != Some("rs") {
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

// ─── G8: observability artifact + PARITY_REPORT present ────────────────────

#[test]
fn gate_8_observability_and_report_present() {
    let obs: PathBuf = [env!("CARGO_MANIFEST_DIR"), "observability.toml"]
        .iter()
        .collect();
    let obs_text = fs::read_to_string(&obs).expect("observability.toml must exist");
    let panel_count = obs_text.matches("[[panel]]").count();
    let alert_count = obs_text.matches("[[alert]]").count();
    assert!(
        panel_count >= 8,
        "observability.toml needs >= 8 panels (got {})",
        panel_count
    );
    assert!(
        alert_count >= 5,
        "observability.toml needs >= 5 alerts (got {})",
        alert_count
    );
    let report: PathBuf = [env!("CARGO_MANIFEST_DIR"), "PARITY_REPORT.md"]
        .iter()
        .collect();
    assert!(report.exists(), "PARITY_REPORT.md must exist");
}

// ─── G9: identity public surface intact ────────────────────────────────────

#[test]
fn gate_9_identity_surface_intact() {
    use cave_identity::agent_manager::{AgentManager, SdsSecret};
    use cave_identity::attestor::{
        AttestorEngine, DockerWorkloadAttestor, UnixProcessInfo, UnixWorkloadAttestor,
        X509PopAttestor,
    };
    use cave_identity::bundle::{self, BundleDoc};
    use cave_identity::error::IdentityError;
    use cave_identity::federation::{FederationManager, StubBundleFetcher};
    use cave_identity::jwt_svid;
    use cave_identity::k8s_attestor::{K8sPodInfo, K8sPsatNodeAttestor, K8sWorkloadAttestor};
    use cave_identity::models::{
        Bundle, BundleEndpointProfile, FederationRelationship, RegistrationEntry, Selector,
        SpiffeId, TrustDomain,
    };
    use cave_identity::oidc::{self, OidcDiscovery};
    use cave_identity::policy::{admit_entry, Caller, PolicyConfig};
    use cave_identity::registration::{
        selectors_equal, selectors_match, selectors_superset, InMemoryEntryStore,
    };
    use cave_identity::routes::{create_router, IdentityState};
    use cave_identity::server_ca::{RotationParams, ServerCa, SignatureAlgorithm};
    use cave_identity::spiffe_id::{agent_id, is_descendant, parse_spiffe_id};
    use cave_identity::store::{MemStore, SqliteStoreFacade};
    use cave_identity::x509_svid;
    use chrono::Utc;
    use std::sync::Arc;

    // 1. SPIFFE-ID parser
    let parsed = parse_spiffe_id("spiffe://example.org/svc").unwrap();
    assert_eq!(parsed.trust_domain.as_str(), "example.org");
    assert!(is_descendant(
        &SpiffeId::new("spiffe://example.org/team"),
        &SpiffeId::new("spiffe://example.org/team/svc"),
    ));
    let agent = agent_id(&TrustDomain::new("example.org"), "k8s_psat", "n1").unwrap();
    assert_eq!(agent.as_str(), "spiffe://example.org/spire/agent/k8s_psat/n1");

    // 2. Server CA bootstrap + rotation
    let ca = ServerCa::new(TrustDomain::new("example.org"), RotationParams::default());
    ca.bootstrap(Utc::now()).unwrap();
    ca.rotate_intermediate(Utc::now()).unwrap();
    ca.rotate_jwt_key(Utc::now()).unwrap();
    let bundle_ = ca.trust_bundle();
    assert!(!bundle_.x509_authorities.is_empty());
    assert!(!bundle_.jwt_authorities.is_empty());
    let _ = SignatureAlgorithm::EcdsaP256Sha256.jwk_kty();

    // 3. Bundle JWKS round-trip
    let doc = bundle::marshal(&bundle_);
    let restored = bundle::unmarshal(&bundle_.trust_domain, &doc).unwrap();
    assert_eq!(restored.sequence_number, bundle_.sequence_number);

    // 4. Registration entries
    let store_entries = InMemoryEntryStore::new();
    let entry = store_entries
        .create(RegistrationEntry {
            spiffe_id: SpiffeId::new("spiffe://example.org/svc"),
            parent_id: SpiffeId::new("spiffe://example.org/spire/agent/k8s_psat/n1"),
            selectors: vec![Selector::new("k8s", "ns:default")],
            ..Default::default()
        })
        .unwrap();
    assert!(!entry.id.is_empty());
    assert!(selectors_match(
        &[Selector::new("k8s", "ns:default")],
        &entry.selectors,
    ));
    assert!(selectors_equal(&entry.selectors, &entry.selectors));
    assert!(selectors_superset(&entry.selectors, &entry.selectors));

    // 5. Policy admission
    let cfg = PolicyConfig::new(TrustDomain::new("example.org"));
    let admin = Caller {
        spiffe_id: SpiffeId::new("spiffe://example.org/admin"),
        admin: true,
    };
    admit_entry(&cfg, &admin, entry.clone()).unwrap();

    // 6. SVID issue + verify
    let svid = x509_svid::issue(&ca, &entry).unwrap();
    let int = ca.current_intermediate().unwrap();
    let bundle_for_verify = Bundle {
        trust_domain: TrustDomain::new("example.org"),
        x509_authorities: vec![cave_identity::models::X509Authority {
            asn1_der: int.cert_der.clone(),
            tainted: false,
        }],
        jwt_authorities: vec![],
        refresh_hint_seconds: 60,
        sequence_number: 1,
    };
    x509_svid::verify(&svid, &bundle_for_verify).unwrap();

    let jwt = jwt_svid::issue(&ca, &entry, vec!["api.example".to_string()]).unwrap();
    let claims = jwt_svid::verify(&jwt.token, "api.example", &ca.trust_bundle()).unwrap();
    assert_eq!(claims.sub, "spiffe://example.org/svc");

    // 7. Attestors
    let eng = AttestorEngine::new();
    let unix = Arc::new(UnixWorkloadAttestor::default());
    unix.table.insert(
        1,
        UnixProcessInfo {
            uid: 0,
            gid: 0,
            path: "/u".into(),
            sha256: None,
        },
    );
    eng.register_workload(unix);
    let _ = DockerWorkloadAttestor::default();
    let _ = X509PopAttestor::default();
    let k8s = K8sWorkloadAttestor::default();
    k8s.register(99, K8sPodInfo::default());
    let psat = K8sPsatNodeAttestor::new("example.org", "c");
    assert_eq!(psat.cluster, "c");

    // 8. Federation
    let store = Arc::new(MemStore::new());
    let fed = FederationManager::new(store.clone(), Arc::new(StubBundleFetcher::default()));
    fed.create(FederationRelationship {
        trust_domain: TrustDomain::new("peer.org"),
        bundle_endpoint_url: "https://peer.org/bundle".into(),
        bundle_endpoint_profile: BundleEndpointProfile::HttpsWeb,
        trust_domain_bundle: None,
    })
    .unwrap();
    assert_eq!(fed.list().len(), 1);

    // 9. OIDC discovery + JWKS
    let disc = OidcDiscovery::new("https://spire.example.org");
    assert_eq!(disc.jwks_uri, "https://spire.example.org/keys");
    let jwks = oidc::jwks_for_bundle(&ca.trust_bundle());
    assert!(!jwks.keys.is_empty());

    // 10. Agent manager + SDS
    let ca_arc = Arc::new(ServerCa::new(
        TrustDomain::new("example.org"),
        RotationParams::default(),
    ));
    ca_arc.bootstrap(Utc::now()).unwrap();
    let mgr = AgentManager::new(ca_arc.clone());
    mgr.bootstrap(&[entry.clone()]).unwrap();
    let _: SdsSecret = mgr.sds_fetch(&entry.spiffe_id).unwrap();

    // 11. Routes builds
    let state = Arc::new(IdentityState {
        ca: ca_arc,
        store: store.clone(),
        federation: Arc::new(fed),
        agents: Arc::new(mgr),
        issuer_url: "https://spire.example.org".into(),
    });
    let _ = create_router(state);

    // 12. SQLite facade
    let facade = SqliteStoreFacade::new("sqlite::memory:");
    assert!(facade.list_entries().is_empty());

    // 13. Error variants exhaustive — surface check
    let _ = IdentityError::EntryNotFound("x".into());
    let _ = BundleDoc {
        keys: vec![],
        spiffe_refresh_hint: 0,
        spiffe_sequence: 0,
    };
}

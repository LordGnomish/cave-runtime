// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Charter v2 self-audit — cave-cert-manager must carry an honest,
//! measured `fill_ratio` against upstream cert-manager/cert-manager
//! v1.20.2, a pinned `source_sha`, the 2026-05-23 v2 re-run audit
//! date, `parity_ratio_source = "manifest"`, 100% AGPL SPDX header
//! coverage, no stub macros in `src/`, mapped+partial+skipped+unmapped
//! summing to total, the full cert-manager public surface reachable
//! through `cave_cert_manager`, the metrics exposition emits the
//! upstream 5-family registry, and the revocation ledger round-trips
//! every RFC 5280 reasonCode.
//!
//! 11 assertions — one per gate of the close-out checklist, plus
//! one each for the two v2 add-ons (metrics + revocation).

use std::fs;
use std::path::PathBuf;

const TODAY: &str = "2026-05-23";
const FLOOR_FILL_RATIO: f64 = 0.95;
const PINNED_VERSION: &str = "v1.20.2";
const PINNED_SHA: &str = "e5b7b18450dd2c4b993b95bcd680b1a057205b00";

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

// ─── Assertion 1: upstream pinned to v1.20.2 ────────────────────────────────

#[test]
fn assertion_1_upstream_version_pinned() {
    let m = manifest_text();
    let v = extract_after(&m, "\nversion ").or_else(|| extract_after(&m, "\nversion="));
    assert_eq!(
        v.as_deref(),
        Some(PINNED_VERSION),
        "[upstream] version must pin cert-manager {} — Charter v2 always-latest gate (got {:?})",
        PINNED_VERSION,
        v
    );
}

// ─── Assertion 2: source_sha matches commit for v1.20.2 ─────────────────────

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
        "source_sha must match the v1.20.2 tag commit (got {:?})",
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
        "cave-cert-manager MVP floor: fill_ratio must be >= {} (got {})",
        FLOOR_FILL_RATIO,
        ratio
    );
    assert!(ratio <= 1.0, "fill_ratio must be a fraction (got {})", ratio);
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

// ─── Assertion 5: last_audit == 2026-05-22 ──────────────────────────────────

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
        "cave-cert-manager MVP floor: >= 15 mapped cert-manager subsystems (got {})",
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
        total >= 15,
        "expected >= 15 .rs files in cave-cert-manager (14 modules + ≥1 test); got {}",
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

// ─── Assertion 9: cert-manager surface intact ──────────────────────────────

#[test]
fn assertion_9_cert_manager_surface_intact() {
    use cave_cert_manager::cli;
    use cave_cert_manager::controller::{
        CertControlPlane, ReconcileEvent,
    };
    use cave_cert_manager::issuer::{IssuerRegistry, IssueOutcome};
    use cave_cert_manager::models::{
        AcmeChallengeSolver, AcmeSolver, Certificate, CertificateConditionType,
        CertificateSpec, ClusterIssuer, ConditionStatus, DnsProvider, IssuerKind, IssuerRef,
        IssuerRefKind, IssuerResource, IssuerSpec, KeyAlgo, KeyEncoding, KeySize,
        PrivateKeyPolicy, RotationPolicy, Usage,
    };
    use cave_cert_manager::renewal::{RenewalReason, RenewalScheduler};
    use cave_cert_manager::secret::SecretMaterializer;
    use cave_cert_manager::store::CertManagerStore;
    use cave_cert_manager::{
        CertManagerError, UPSTREAM_SOURCE_SHA, UPSTREAM_VERSION,
    };
    use chrono::{Duration, Utc};
    use std::collections::BTreeMap;
    use uuid::Uuid;

    // 1. Pinned constants are reachable + match the manifest.
    assert_eq!(UPSTREAM_VERSION, "v1.20.2");
    assert_eq!(UPSTREAM_SOURCE_SHA, "e5b7b18450dd2c4b993b95bcd680b1a057205b00");

    // 2. Every IssuerKind variant is constructible.
    let _ = IssuerKind::Acme;
    let _ = IssuerKind::Ca;
    let _ = IssuerKind::Vault;
    let _ = IssuerKind::SelfSigned;
    let _ = IssuerKind::Venafi;

    // 3. Every CertificateConditionType variant is constructible.
    let _ = CertificateConditionType::Ready;
    let _ = CertificateConditionType::Issuing;
    let _ = ConditionStatus::True;
    let _ = ConditionStatus::False;
    let _ = ConditionStatus::Unknown;

    // 4. PrivateKeyPolicy defaults are reachable + sensible.
    let pkp = PrivateKeyPolicy::default();
    assert!(matches!(pkp.rotation, RotationPolicy::Never));
    assert!(matches!(pkp.algorithm, KeyAlgo::Ecdsa));
    assert!(matches!(pkp.size, KeySize::Ecdsa256));
    assert!(matches!(pkp.encoding, KeyEncoding::Pkcs8));

    // 5. CertManagerStore + IssuerRegistry + SecretMaterializer compose.
    let mut store = CertManagerStore::new();
    let mut registry = IssuerRegistry::new();
    let mut secrets = SecretMaterializer::new();
    assert!(registry.supports(IssuerKind::Acme));
    assert!(registry.supports(IssuerKind::Ca));
    assert!(registry.supports(IssuerKind::Vault));
    assert!(registry.supports(IssuerKind::SelfSigned));

    // 6. End-to-end reconcile through CertControlPlane against a
    //    SelfSigned cluster-issuer yields a Ready Certificate.
    let mut cp = CertControlPlane::new();
    cp.store.put_cluster_issuer(ClusterIssuer {
        id: Uuid::new_v4(),
        name: "selfsigned".into(),
        tenant_id: "t-1".into(),
        spec: IssuerSpec::SelfSigned {
            crl_distribution_points: vec![],
        },
        created_at: Utc::now(),
    });
    let cert_id = cp.store.put_certificate(Certificate {
        id: Uuid::new_v4(),
        name: "audit-9".into(),
        namespace: "default".into(),
        tenant_id: "t-1".into(),
        spec: CertificateSpec {
            secret_name: "audit-9-tls".into(),
            issuer_ref: IssuerRef {
                name: "selfsigned".into(),
                kind: IssuerRefKind::ClusterIssuer,
                group: "cert-manager.io".into(),
            },
            dns_names: vec!["audit-9.example.com".into()],
            ip_addresses: vec![],
            uris: vec![],
            email_addresses: vec![],
            common_name: None,
            duration_seconds: 90 * 24 * 3600,
            renew_before_seconds: 30 * 24 * 3600,
            usages: vec![Usage::ServerAuth],
            private_key: PrivateKeyPolicy::default(),
            is_ca: false,
            subject: None,
            secret_template_labels: BTreeMap::new(),
            secret_template_annotations: BTreeMap::new(),
        },
        status: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        labels: BTreeMap::new(),
        annotations: BTreeMap::new(),
    });
    let res = cp.controller().reconcile("t-1", cert_id).unwrap();
    assert_eq!(res.new_revision, 1);
    assert!(matches!(res.events[0], ReconcileEvent::Issued { .. }));

    let cert_ro = cp.store.certificate("t-1", cert_id).unwrap();
    let status = cert_ro.status.as_ref().unwrap();
    assert_eq!(status.conditions[0].status, ConditionStatus::True);

    // 7. RenewalScheduler returns InitialIssuance for a fresh cert.
    let fresh = cert_with_no_status();
    let plan = RenewalScheduler::evaluate(&fresh, Utc::now()).unwrap();
    assert_eq!(plan.reason, RenewalReason::InitialIssuance);

    // 8. ACME solver pick + DnsProvider + AcmeChallengeSolver compose;
    //    IssueOutcome shape is constructible.
    let solver = AcmeSolver {
        dns_zones: vec!["example.com".into()],
        challenge: AcmeChallengeSolver::Dns01 {
            provider: DnsProvider::CaveDns {
                zone: "example.com.".into(),
            },
        },
    };
    let _ = solver.clone();
    let _outcome = IssueOutcome {
        certificate_chain_pem: "x".into(),
        ca_pem: "y".into(),
        not_before: Utc::now(),
        not_after: Utc::now() + Duration::seconds(1),
        serial: "deadbeef".into(),
    };

    // 9. cli URL builders match the route surface + IssuerResource is
    //    constructible + tenant scoping error round-trips.
    assert_eq!(cli::health_path(), "/api/cert/health");
    assert!(cli::certificates_path("t-1").contains("/api/cert/t-1/"));
    let _ = IssuerResource {
        id: Uuid::new_v4(),
        name: "in-ns".into(),
        namespace: "ns".into(),
        tenant_id: "t-1".into(),
        spec: IssuerSpec::SelfSigned {
            crl_distribution_points: vec![],
        },
        created_at: Utc::now(),
    };
    assert!(matches!(
        store.certificate("t-1", Uuid::new_v4()),
        Err(CertManagerError::CertificateNotFound(_))
    ));
    let _ = secrets.len();
}

// ─── Assertion 10: metrics exposition emits the 5 upstream families ────────

#[test]
fn assertion_10_metrics_exposition_includes_all_upstream_families() {
    use cave_cert_manager::metrics::{AcmeRequestLabels, CertManagerMetrics};
    let mut m = CertManagerMetrics::new();
    m.record_sync("certificates");
    m.record_acme_request(AcmeRequestLabels {
        scheme: "https".into(),
        host: "acme.example.com".into(),
        method: "POST".into(),
        status: 200,
    });
    let out = m.render_prometheus();
    for family in [
        "certmanager_certificate_ready_status",
        "certmanager_certificate_expiration_timestamp_seconds",
        "certmanager_certificate_renewal_timestamp_seconds",
        "certmanager_acme_client_request_count",
        "certmanager_controller_sync_call_count",
    ] {
        assert!(
            out.contains(family),
            "metrics exposition missing upstream family `{family}` — Charter v2 observability gate"
        );
    }
}

// ─── Assertion 11: revocation ledger round-trips every RFC 5280 reason ─────

#[test]
fn assertion_11_revocation_ledger_round_trips_rfc5280_reasoncodes() {
    use cave_cert_manager::revocation::RevocationReason;
    let valid = [0u8, 1, 2, 3, 4, 5, 6, 8, 9, 10];
    for code in valid {
        let r = RevocationReason::from_reason_code(code)
            .unwrap_or_else(|_| panic!("RFC 5280 reasonCode {code} must round-trip"));
        assert_eq!(r.reason_code(), code);
    }
    assert!(
        RevocationReason::from_reason_code(7).is_err(),
        "reasonCode 7 is RFC 5280 reserved — must NOT round-trip"
    );
    assert!(
        RevocationReason::from_reason_code(42).is_err(),
        "reasonCode 42 is outside the RFC 5280 enumeration — must NOT round-trip"
    );
}

fn cert_with_no_status() -> cave_cert_manager::models::Certificate {
    use cave_cert_manager::models::{
        Certificate, CertificateSpec, IssuerRef, IssuerRefKind, PrivateKeyPolicy,
    };
    use chrono::Utc;
    use std::collections::BTreeMap;
    use uuid::Uuid;

    Certificate {
        id: Uuid::new_v4(),
        name: "f".into(),
        namespace: "default".into(),
        tenant_id: "t-1".into(),
        spec: CertificateSpec {
            secret_name: "tls".into(),
            issuer_ref: IssuerRef {
                name: "selfsigned".into(),
                kind: IssuerRefKind::ClusterIssuer,
                group: "cert-manager.io".into(),
            },
            dns_names: vec!["x.example.com".into()],
            ip_addresses: vec![],
            uris: vec![],
            email_addresses: vec![],
            common_name: None,
            duration_seconds: 3600,
            renew_before_seconds: 600,
            usages: vec![],
            private_key: PrivateKeyPolicy::default(),
            is_ca: false,
            subject: None,
            secret_template_labels: BTreeMap::new(),
            secret_template_annotations: BTreeMap::new(),
        },
        status: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        labels: BTreeMap::new(),
        annotations: BTreeMap::new(),
    }
}

fn walk(p: &PathBuf, visit: &mut dyn FnMut(&PathBuf)) {
    let Ok(entries) = fs::read_dir(p) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // skip target/ to keep the audit local-only
            if path.file_name().map(|f| f == "target").unwrap_or(false) {
                continue;
            }
            walk(&path, visit);
        } else {
            visit(&path);
        }
    }
}

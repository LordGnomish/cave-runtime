// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Charter v2 self-audit — cave-k8s must carry an honest, measured
//! `fill_ratio` against upstream kubernetes/kubernetes v1.32.0, a
//! pinned `source_sha` for reproducibility, the 2026-05-23 close-out
//! audit date, `parity_ratio_source = "manifest"`, 100% AGPL SPDX
//! header coverage, no stub macros in `src/`, mapped+partial+skipped+
//! unmapped summing to total, and the full ControlPlane / admission /
//! authn / authz / PQC SA-token surface reachable through `cave_k8s`.
//!
//! 9 assertions — one per gate of the close-out checklist.

use std::fs;
use std::path::PathBuf;

const TODAY: &str = "2026-05-23";
const FLOOR_FILL_RATIO: f64 = 0.95;
const K8S_VERSION: &str = "v1.32.0";
const K8S_SHA: &str = "70d3cc986aa8221cd1dfb1121852688902d3bf53";

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

// ─── Assertion 1: kubernetes upstream pinned to v1.32.0 ────────────────────

#[test]
fn assertion_1_kubernetes_version_pinned() {
    let m = manifest_text();
    let v = extract_after(&m, "\nversion ").or_else(|| extract_after(&m, "\nversion="));
    assert_eq!(
        v.as_deref(),
        Some(K8S_VERSION),
        "[upstream] version must pin Kubernetes {} — Charter v2 always-latest gate (got {:?})",
        K8S_VERSION,
        v
    );
}

// ─── Assertion 2: source_sha matches v1.32.0 ───────────────────────────────

#[test]
fn assertion_2_source_sha_matches_version() {
    let m = manifest_text();
    assert!(
        m.contains(K8S_SHA),
        "[upstream] kubernetes source_sha must contain {} (full manifest text scan)",
        K8S_SHA
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
        "cave-k8s Charter v2 floor: fill_ratio must be >= {} (got {})",
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

// ─── Assertion 6: counts sum to total + >= 25 mapped ────────────────────────

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
        mapped >= 25,
        "cave-k8s umbrella floor: >= 25 mapped K8s subsystems (got {})",
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
        total >= 25,
        "expected >= 25 .rs files in cave-k8s; got {}",
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

// ─── Assertion 9: ControlPlane / admission / authn / authz surface intact ──

#[test]
fn assertion_9_control_plane_surface_intact() {
    use cave_k8s::admission::{Chain, LimitRanger, NamespaceLifecycle, Operation, PodSecurityRestricted, Request, ServiceAccountDefaulter};
    use cave_k8s::aggregator::{AggregatorRegistry, ApiService};
    use cave_k8s::authn::{
        Authenticator, BootstrapTokenAuthenticator, ChainAuthenticator, OidcAuthenticator,
        ServiceAccountAuthenticator, X509ClientCertAuthenticator,
    };
    use cave_k8s::authz::{
        Attributes, Binding, ChainAuthorizer, NodeAuthorizer, PolicyRule as RbacRule,
        RbacAuthorizer, Role, Subject, SubjectKind, Verb, WebhookAuthorizer,
    };
    use cave_k8s::cgroup::{classify_qos, pod_cgroup_path, QosClass};
    use cave_k8s::cluster::{ClusterConfig, ControlPlane};
    use cave_k8s::crd::{Crd, CrdRegistry, CrdVersion, Scope};
    use cave_k8s::discovery::Discovery;
    use cave_k8s::eviction::{plan_eviction, EvictionCandidate, PressureObservation, PressureSignal, PressureThreshold};
    use cave_k8s::garbage_collector::{GarbageCollector, OwnerEdge, Propagation};
    use cave_k8s::images::{plan_image_gc, GcPolicy, ImageRecord};
    use cave_k8s::kubelet_facade::{drive_pod_action, LifecycleAction, NodeStatus, PodAssignment, PodPhase};
    use cave_k8s::models::{BuiltinKind, ClusterPhase, ComponentName, NodeRole, ResourceRef};
    use cave_k8s::networking::{derive_slices, EndpointSlice, IpFamily, ServiceType};
    use cave_k8s::observability_metrics::{MetricKind, MetricRegistry};
    use cave_k8s::openapi::OpenApiAggregator;
    use cave_k8s::pqc::{
        sign_sa_jwt, HybridSigner, HybridVerifier, SaClaims, ALG_HYBRID, PQC_SIG_LEN,
    };
    use cave_k8s::probes::{ProbeConfig, ProbeKind, ProbeResult, ProbeState};
    use cave_k8s::proxy_facade::{BackendEntry, ProxyMode, ProxyRegistry};
    use cave_k8s::quota::{Dimension, Quota, QuotaTracker};
    use cave_k8s::resources::Manager;
    use cave_k8s::routes::create_router;
    use cave_k8s::scheduler_facade::{place, NodeCandidate, PlacementOutcome, PlacementRequest};
    use cave_k8s::state::State;
    use cave_k8s::storage::{AccessMode, Binder, PersistentVolume, PersistentVolumeClaim, ReclaimPolicy};
    use cave_k8s::vap::{Policy, PolicyMatch, PolicyPlugin, PolicyRule};
    use cave_k8s::workloads::{plan_rolling_update, CronExpr, RolloutStrategy, WorkloadKind};
    use cave_k8s::{Error, MODULE_NAME, UPSTREAM_SHA, UPSTREAM_VERSION};
    use std::sync::Arc;

    // ── 1. Module identity ─────────────────────────────────────────────────
    assert_eq!(MODULE_NAME, "k8s");
    assert_eq!(UPSTREAM_VERSION, "v1.32.0");
    assert_eq!(UPSTREAM_SHA.len(), 40);

    // ── 2. ControlPlane bootstrap ──────────────────────────────────────────
    let cp = ControlPlane::new(ClusterConfig::default());
    cp.start();
    assert_eq!(cp.phase(), ClusterPhase::Running);
    assert_eq!(cp.status().total_components, 7);

    // ── 3. State + router ──────────────────────────────────────────────────
    let state = Arc::new(State::default());
    let _router = create_router(state.clone());

    // ── 4. PQC hybrid SA tokens roundtrip ──────────────────────────────────
    let signer = HybridSigner::from_seed([1u8; 32]);
    let claims = SaClaims {
        iss: "cave-k8s".into(),
        sub: "system:serviceaccount:default:cave".into(),
        aud: vec!["kube-apiserver".into()],
        exp: 9_999_999_999,
        iat: 0,
        jti: "tok-1".into(),
    };
    let tok = sign_sa_jwt(&signer, &claims);
    assert!(tok.split('.').count() == 3);
    let v = HybridVerifier::new(signer.classical_public()).with_expected_pqc_seed([1u8; 32]);
    let parts: Vec<&str> = tok.split('.').collect();
    let env = base64::Engine::decode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, parts[2])
        .unwrap();
    let input = format!("{}.{}", parts[0], parts[1]);
    v.verify(input.as_bytes(), &env).unwrap();
    assert_eq!(ALG_HYBRID, "Ed25519+ML-DSA-65");
    assert_eq!(PQC_SIG_LEN, 3309);

    // ── 5. Admission chain ─────────────────────────────────────────────────
    let chain = Chain::new()
        .add(Box::new(NamespaceLifecycle::default()))
        .add(Box::new(ServiceAccountDefaulter))
        .add(Box::new(LimitRanger {
            min_cpu_request_millis: 1,
        }))
        .add(Box::new(PodSecurityRestricted));
    let mut req = Request {
        operation: Operation::Create,
        namespace: "default".into(),
        kind: "Pod".into(),
        name: "p".into(),
        user: "alice".into(),
        object: serde_json::json!({
            "spec": {"containers": [{"resources": {"requests": {"cpu": "100m"}}}]}
        }),
    };
    chain.admit(&mut req).unwrap();
    assert_eq!(
        req.object
            .get("spec")
            .and_then(|s| s.get("serviceAccountName"))
            .and_then(|v| v.as_str()),
        Some("default")
    );

    // ── 6. Authn chain (SA token + X.509 + OIDC + bootstrap) ───────────────
    let sa_auth = ServiceAccountAuthenticator::new("cave-k8s");
    sa_auth.register("tok-1", "default", "cave");
    let id = sa_auth.authenticate(&tok).unwrap();
    assert!(id.user.starts_with("system:serviceaccount:"));
    let x509 = X509ClientCertAuthenticator::default();
    x509.add_cn("alice", vec!["system:masters".into()]);
    let chain_authn = ChainAuthenticator::new()
        .add(Box::new(BootstrapTokenAuthenticator::default()))
        .add(Box::new(OidcAuthenticator::new("https://oidc", "kube")))
        .add(Box::new(x509));
    let aid = chain_authn.authenticate("x509://alice").unwrap();
    assert_eq!(aid.user, "alice");

    // ── 7. Authz chain (RBAC + Node + Webhook) ─────────────────────────────
    let rbac = RbacAuthorizer::default();
    rbac.add_role(Role {
        name: "viewer".into(),
        namespace: Some("default".into()),
        rules: vec![RbacRule {
            api_groups: vec!["".into()],
            resources: vec!["pods".into()],
            resource_names: vec![],
            verbs: vec![Verb::Get],
        }],
    });
    rbac.bind(Binding {
        name: "alice-view".into(),
        namespace: Some("default".into()),
        role_name: "viewer".into(),
        cluster_role: false,
        subjects: vec![Subject {
            kind: SubjectKind::User,
            name: "alice".into(),
        }],
    });
    let chain_authz = ChainAuthorizer::new()
        .add(Box::new(WebhookAuthorizer::default()))
        .add(Box::new(NodeAuthorizer))
        .add(Box::new(rbac));
    chain_authz
        .authorize(&Attributes {
            user: aid,
            verb: Verb::Get,
            api_group: "".into(),
            resource: "pods".into(),
            namespace: Some("default".into()),
            name: None,
        })
        .unwrap();

    // ── 8. CRD + aggregator + discovery + openapi ──────────────────────────
    let crds = Arc::new(CrdRegistry::new());
    crds.install(Crd {
        group: "cave.example.com".into(),
        plural: "widgets".into(),
        kind: "Widget".into(),
        scope: Scope::Namespaced,
        versions: vec![CrdVersion {
            name: "v1".into(),
            served: true,
            storage: true,
            schema: serde_json::json!({"type": "object"}),
        }],
    })
    .unwrap();
    let aggr = Arc::new(AggregatorRegistry::new());
    aggr.register(ApiService {
        name: "v1.metrics.k8s.io".into(),
        group: "metrics.k8s.io".into(),
        version: "v1".into(),
        service: "kube-system/metrics:443".into(),
        insecure_skip_tls_verify: false,
        group_priority_minimum: 100,
        version_priority: 10,
    });
    aggr.mark_available("v1.metrics.k8s.io");
    let disc = Discovery::new(crds.clone(), aggr).doc();
    assert!(disc.groups.iter().any(|g| g.name == "metrics.k8s.io"));
    assert!(disc.groups.iter().any(|g| g.name == "cave.example.com"));
    let oa = OpenApiAggregator::new(crds).compose();
    assert_eq!(oa.openapi, "3.0.0");

    // ── 9. GC + quota + storage + scheduler + kubelet + proxy + cgroup ────
    let gc = GarbageCollector::new();
    gc.link(OwnerEdge {
        owner: ResourceRef::namespaced("Deployment", "default", "d"),
        child: ResourceRef::namespaced("ReplicaSet", "default", "r"),
        block: true,
    });
    let plan = gc.cascade_plan(
        &ResourceRef::namespaced("Deployment", "default", "d"),
        Propagation::Foreground,
    );
    assert!(plan.len() >= 2);

    let q = QuotaTracker::new();
    q.install(Quota::new("default", "pods").with_limit(Dimension::Pods, 1));
    q.admit_and_commit("default", &Dimension::Pods, 1).unwrap();
    assert!(q
        .check_admit("default", &Dimension::Pods, 1)
        .is_err());

    let b = Binder::new();
    b.add_pv(PersistentVolume {
        name: "pv1".into(),
        storage_class: "gp3".into(),
        capacity_bytes: 1024,
        access_modes: vec![AccessMode::ReadWriteOnce],
        reclaim_policy: ReclaimPolicy::Delete,
        csi_driver: "csi.cave".into(),
        volume_handle: "vol/pv1".into(),
        phase: cave_k8s::storage::BindingPhase::Available,
        claim: None,
    });
    b.add_pvc(PersistentVolumeClaim {
        namespace: "default".into(),
        name: "c1".into(),
        requested_bytes: 512,
        access_modes: vec![AccessMode::ReadWriteOnce],
        storage_class: Some("gp3".into()),
        bound_to: None,
        phase: cave_k8s::storage::BindingPhase::Pending,
    });
    assert_eq!(b.bind_once(), 1);

    let sched = place(
        &PlacementRequest {
            namespace: "default".into(),
            pod_name: "p".into(),
            scheduler_name: "default-scheduler".into(),
            cpu_request_millis: 100,
            memory_request_bytes: 0,
            node_selector: Default::default(),
            tolerations: vec![],
        },
        &[NodeCandidate {
            name: "n1".into(),
            cpu_allocatable_millis: 4000,
            memory_allocatable_bytes: 1 << 30,
            labels: Default::default(),
            taints: vec![],
        }],
    );
    assert!(matches!(sched, PlacementOutcome::Bound { .. }));

    let mut pod = PodAssignment {
        namespace: "default".into(),
        name: "p".into(),
        uid: "u1".into(),
        node: "n1".into(),
        phase: PodPhase::Pending,
        started_at: chrono::Utc::now(),
        restart_count: 0,
    };
    let new_phase = drive_pod_action(&mut pod, LifecycleAction::Start);
    assert_eq!(new_phase, PodPhase::Running);

    let pr = ProxyRegistry::new(ProxyMode::Nftables);
    pr.upsert(BackendEntry {
        service: "svc".into(),
        namespace: "default".into(),
        virtual_ip: "10.0.0.1".into(),
        virtual_port: 80,
        backends: vec![("10.244.0.1".into(), 8080)],
        session_affinity: false,
    });
    assert_eq!(pr.count(), 1);

    let qos = classify_qos(&[(500, 1024 * 1024, 500, 1024 * 1024)]);
    assert_eq!(qos, QosClass::Guaranteed);
    let cg = pod_cgroup_path(QosClass::Guaranteed, "abc-def");
    assert!(cg.as_str().contains("podsguaranteed"));

    // ── 10. probes + eviction + image gc + workloads ──────────────────────
    let mut ps = ProbeState::new(ProbeKind::Liveness);
    let cfg = ProbeConfig::liveness_default();
    ps.record(ProbeResult::Failure, &cfg);
    ps.record(ProbeResult::Failure, &cfg);
    ps.record(ProbeResult::Failure, &cfg);
    assert!(!ps.passing);

    let plan = plan_eviction(
        &[PressureThreshold {
            signal: PressureSignal::MemoryAvailable,
            minimum_available_bytes: 500,
            grace_period_seconds: 0,
        }],
        &[PressureObservation {
            signal: PressureSignal::MemoryAvailable,
            available_bytes: 100,
        }],
        vec![EvictionCandidate {
            namespace: "x".into(),
            name: "p1".into(),
            usage: 50,
            priority: 0,
        }],
    );
    assert_eq!(plan.len(), 1);

    let img_plan = plan_image_gc(
        &GcPolicy::default(),
        950,
        1000,
        std::time::SystemTime::now(),
        vec![ImageRecord {
            id: "old".into(),
            size_bytes: 100,
            used_by: 0,
            last_used: std::time::SystemTime::now() - std::time::Duration::from_secs(1000),
        }],
    );
    assert_eq!(img_plan.len(), 1);

    let rs = plan_rolling_update(5, 1, 2, 0);
    assert!(rs.len() >= 2);
    assert!(CronExpr::parse("0 0 * * *").is_some());
    let _ = RolloutStrategy::Canary { percent: 25 };
    let _ = WorkloadKind::Deployment;

    // ── 11. networking + resources + metrics ──────────────────────────────
    let slices: Vec<EndpointSlice> = derive_slices(
        "default",
        "svc",
        IpFamily::Ipv4,
        vec![],
        &[("p".into(), "n1".into(), "10.244.0.1".into(), true)],
        10,
    );
    assert_eq!(slices.len(), 1);
    let _ = ServiceType::ClusterIP;

    let mgr = Manager::new(state.apiserver.clone());
    let _ = mgr.counts();
    let _ = mgr.namespaces();

    let metrics = MetricRegistry::new();
    assert_eq!(metrics.names().len(), 7);
    metrics.set_gauge("cave_k8s_pod_count", &[("ns", "default"), ("phase", "Running")], 3.0);
    let scrape = metrics.scrape_text();
    assert!(scrape.contains("cave_k8s_pod_count"));
    assert_eq!(MetricKind::Counter, MetricKind::Counter);

    // ── 12. VAP + Error type + builtin kinds + components ─────────────────
    let policy = Policy {
        name: "require-sa".into(),
        validating: true,
        match_rules: PolicyMatch {
            kinds: vec!["Pod".into()],
            operations: vec![Operation::Create],
        },
        rules: vec![PolicyRule {
            name: "sa-required".into(),
            expression: "has(object.spec.serviceAccountName)".into(),
            message: "set it".into(),
        }],
    };
    let plug = PolicyPlugin { policy };
    let mut r = Request {
        operation: Operation::Create,
        namespace: "default".into(),
        kind: "Pod".into(),
        name: "x".into(),
        user: "alice".into(),
        object: serde_json::json!({"spec": {}}),
    };
    use cave_k8s::admission::{Decision, Plugin};
    assert!(matches!(plug.evaluate(&mut r), Decision::Deny(_)));

    let _ = Error::Forbidden("x".into());
    assert_eq!(ComponentName::ALL.len(), 8);
    assert!(BuiltinKind::Pod.is_namespaced());
    assert!(!BuiltinKind::Namespace.is_namespaced());
    assert_eq!(NodeRole::Hybrid as u8, NodeRole::Hybrid as u8);
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

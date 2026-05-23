// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Charter v2 self-audit — cave-sign must carry an honest, measured
//! `fill_ratio` against upstream sigstore/cosign v3.0.6 + sigstore v1.10.6,
//! pinned `source_sha`s for reproducibility, the 2026-05-22 close-out
//! audit date, `parity_ratio_source = "manifest"`, 100% AGPL SPDX header
//! coverage, no stub macros in `src/`, mapped+partial+skipped+unmapped
//! summing to total, and the full sign/verify/attest/policy/keyless
//! surface reachable through `cave_sign`.
//!
//! 9 assertions — one per gate of the close-out checklist.

use std::fs;
use std::path::PathBuf;

const TODAY: &str = "2026-05-22";
const FLOOR_FILL_RATIO: f64 = 0.65;
const COSIGN_VERSION: &str = "v3.0.6";
const COSIGN_SHA: &str = "f1ad3ee952313be5d74a49d67ba0aa8d0d5e351f";
const SIGSTORE_VERSION: &str = "v1.10.6";
const SIGSTORE_SHA: &str = "311895e7870187320e47337734a9c321c0a8819c";

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

// ─── Assertion 1: cosign upstream pinned to v3.0.6 ──────────────────────────

#[test]
fn assertion_1_cosign_version_pinned() {
    let m = manifest_text();
    let v = extract_after(&m, "\nversion ").or_else(|| extract_after(&m, "\nversion="));
    assert_eq!(
        v.as_deref(),
        Some(COSIGN_VERSION),
        "[upstream] version must pin Cosign {} — Charter v2 always-latest gate (got {:?})",
        COSIGN_VERSION,
        v
    );
}

// ─── Assertion 2: source_sha matches cosign v3.0.6 + sigstore v1.10.6 ───────

#[test]
fn assertion_2_source_sha_matches_versions() {
    let m = manifest_text();
    assert!(
        m.contains(COSIGN_SHA),
        "[upstream] cosign source_sha must contain {} (full manifest text scan)",
        COSIGN_SHA
    );
    assert!(
        m.contains(SIGSTORE_VERSION),
        "[upstreams] companion sigstore version {} must be pinned",
        SIGSTORE_VERSION
    );
    assert!(
        m.contains(SIGSTORE_SHA),
        "[upstreams] companion sigstore source_sha {} must be pinned",
        SIGSTORE_SHA
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
        "cave-sign MVP floor: fill_ratio must be >= {} (got {})",
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
        "cave-sign MVP floor: >= 15 mapped cosign subsystems (got {})",
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
        "expected >= 15 .rs files in cave-sign; got {}",
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

// ─── Assertion 9: full sign / verify / attest / policy surface intact ───────

#[test]
fn assertion_9_cosign_surface_intact() {
    use cave_sign::attestation::{
        DsseEnvelope, build_slsa_provenance, build_vex_predicate, sign_attestation,
        subject_sha256, verify_envelope, VexStatement, VexStatus,
    };
    use cave_sign::blob::{sign_blob_keypair, sign_blob_keypair_with_rekor, verify_blob};
    use cave_sign::bundle::{BundleTriple, CosignBundle};
    use cave_sign::fulcio::FulcioClient;
    use cave_sign::keyless::KeylessSigner;
    use cave_sign::models::{
        Attestation, ArtifactType, KeyAlgorithm, PredicateType, SigKind, SignedArtifact, Signature,
    };
    use cave_sign::oci::{ImageRef, sign_image_keypair, sign_image_keypair_with_rekor, verify_image};
    use cave_sign::oidc::{build_fixture_jwt, IdToken};
    use cave_sign::policy::{extract_claims, glob_match, Policy, Rule};
    use cave_sign::rekor::{HashedRekordEntry, RekorClient, RekorKind};
    use cave_sign::signature::{Keypair, sha256_digest_string};
    use cave_sign::signing_config::{Deployment, SigningConfig};
    use cave_sign::store::SignedArtifactStore;
    use cave_sign::trustedroot::TrustedRoot;
    use cave_sign::verify::{verify, verify_raw_signature, VerifyRequest};
    use cave_sign::{State, router, MODULE_NAME};
    use serde_json::json;
    use std::sync::Arc;

    // ── 1. Module identity + state ─────────────────────────────────────────
    assert_eq!(MODULE_NAME, "sign");
    let _r = router(Arc::new(State::default()));

    // ── 2. Signing primitives — P-256 + Ed25519 roundtrip ──────────────────
    let kp = Keypair::from_seed(KeyAlgorithm::EcdsaP256, &[1u8; 32]).unwrap();
    let sig = kp.sign(b"x").unwrap();
    cave_sign::signature::verify(KeyAlgorithm::EcdsaP256, kp.public_key_bytes(), b"x", &sig).unwrap();
    let kp2 = Keypair::from_seed(KeyAlgorithm::Ed25519, &[2u8; 32]).unwrap();
    let sig2 = kp2.sign(b"y").unwrap();
    cave_sign::signature::verify(KeyAlgorithm::Ed25519, kp2.public_key_bytes(), b"y", &sig2).unwrap();
    assert!(sha256_digest_string(b"").starts_with("sha256:"));

    // ── 3. Blob sign/verify + Rekor binding ────────────────────────────────
    let rk = RekorClient::default();
    let b = sign_blob_keypair(b"hello", &kp).unwrap();
    verify_blob(b"hello", &b.bundle).unwrap();
    let b2 = sign_blob_keypair_with_rekor(b"world", &kp, &rk).unwrap();
    assert!(b2.signature.log_index.is_some());

    // ── 4. OCI image sign + verify ─────────────────────────────────────────
    let img = ImageRef::parse(
        "ghcr.io/cave/runtime@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    )
    .unwrap();
    assert!(img.signature_tag().ends_with(".sig"));
    let (_s, _l, bundle) = sign_image_keypair(&img, 100, &kp).unwrap();
    verify_image(&img, &bundle).unwrap();
    let (sr, _lr, _br) = sign_image_keypair_with_rekor(&img, 100, &kp, &rk).unwrap();
    assert!(sr.log_index.is_some());

    // ── 5. Bundle (re)serialize ────────────────────────────────────────────
    let s = bundle.encode_json().unwrap();
    let back = CosignBundle::decode_json(&s).unwrap();
    assert_eq!(back, bundle);
    let _trip = BundleTriple::from_bundle(&bundle).unwrap();

    // ── 6. Rekor: append + lookup + inclusion proof ────────────────────────
    let entry = HashedRekordEntry {
        digest_hex: "deadbeef".into(),
        signature_b64: "sig".into(),
        public_key_pem: "-----BEGIN PUBLIC KEY-----\nM\n-----END PUBLIC KEY-----".into(),
    };
    let e = rk.upload_offline(entry).unwrap();
    assert_eq!(e.kind, RekorKind::HashedRekord);
    let _proof = rk.inclusion_proof_offline(e.log_index).unwrap();

    // ── 7. Keyless sign through Fulcio mock ────────────────────────────────
    let signer = KeylessSigner::new(FulcioClient::default());
    let token_raw = build_fixture_jwt(&json!({
        "iss":"https://oidc.cave.svc","sub":"alice","aud":"sigstore",
        "exp": chrono::Utc::now().timestamp() + 3600,
        "email":"alice@example.com",
    }));
    let token = IdToken::parse(&token_raw).unwrap();
    let ks = signer.sign_blob(b"keyless-payload", &token, &rk).unwrap();
    assert_eq!(ks.signature.kind, SigKind::Keyless);

    // ── 8. DSSE + SLSA + VEX attestation chain ─────────────────────────────
    let att = Attestation {
        media_type: "application/vnd.in-toto+json".into(),
        predicate_type: PredicateType::SlsaProvenance,
        subject: vec![subject_sha256("ghcr.io/cave/runtime", "deadbeef")],
        predicate: build_slsa_provenance("cave-builder", "cave-build-v1", "https://cave/run/1"),
    };
    let env: DsseEnvelope = sign_attestation(&att, &kp, "keyid-1").unwrap();
    let _back = verify_envelope(&env, KeyAlgorithm::EcdsaP256, kp.public_key_bytes()).unwrap();
    let vex = build_vex_predicate(
        "alice",
        vec![VexStatement {
            vulnerability: "CVE-2026-0001".into(),
            products: vec!["pkg:oci/cave".into()],
            status: VexStatus::NotAffected,
        }],
    );
    assert!(vex["statements"].is_array());

    // ── 9. Policy + cert claims + verify orchestrator ──────────────────────
    let claims = extract_claims(&ks.bundle.cert_pem).unwrap();
    assert_eq!(claims.identity, "alice@example.com");
    let policy = Policy::new("cave-default")
        .require(Rule::CertificateIdentity { glob: "*@example.com".into() })
        .require(Rule::CertificateIssuer { exact: "https://oidc.cave.svc".into() })
        .require(Rule::RequireRekorEntry)
        .require(Rule::RequireKeyless);
    let vr = verify(VerifyRequest {
        payload: b"keyless-payload",
        bundle: &ks.bundle,
        rekor: Some(&rk),
        policy: Some(&policy),
    })
    .unwrap();
    assert_eq!(vr.signer.as_deref(), Some("alice@example.com"));
    verify_raw_signature(b"hello", &b.signature.sig_b64, &b.signature.cert_pem).unwrap();
    assert!(glob_match("*@example.com", "alice@example.com"));

    // ── 10. Trust + signing config ─────────────────────────────────────────
    let root = TrustedRoot::cave_default();
    assert!(!root.fulcio_certs.is_empty());
    let cfg = SigningConfig::sovereign();
    assert_eq!(cfg.deployment, Deployment::Sovereign);
    let cfg2 = SigningConfig::public_good();
    assert!(cfg2.fulcio_url.contains("sigstore.dev"));

    // ── 11. Signature + SignedArtifact + ArtifactType + Store ──────────────
    let _sigkind: Signature = ks.signature.clone();
    let store = SignedArtifactStore::new();
    let a: SignedArtifact = store
        .insert(
            "sha256:00".into(),
            ArtifactType::Sbom,
            "x".into(),
            "alice".into(),
            true,
        )
        .unwrap();
    assert_eq!(store.count().unwrap(), 1);
    assert_eq!(a.signer_identity, "alice");
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

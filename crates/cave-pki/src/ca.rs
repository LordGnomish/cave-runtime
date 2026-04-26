//! CA hierarchy: Root → Platform Intermediate → per-tenant Intermediate
//! → Leaf.
//!
//! Cite: openbao `builtin/logical/pki/backend.go` paths
//! `root/generate/internal`, `intermediate/generate/internal`,
//! `intermediate/set-signed`, `issue`, `sign`. cave-pki keeps the same
//! 3-tier shape with an explicit per-tenant intermediate slot.

use crate::error::{PkiError, PkiResult};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum KeyAlgorithm {
    /// EC NIST P-256.
    EcdsaP256,
    /// EC NIST P-384.
    EcdsaP384,
    /// RSA-2048.
    Rsa2048,
    /// RSA-4096.
    Rsa4096,
    /// Ed25519 (RFC 8410).
    Ed25519,
    /// Hybrid PQC: ML-DSA-65 (FIPS 204) + Ed25519. Cite:
    /// `RFC 9001 §6` discussion + IETF draft-ietf-lamps-pq-composite-sigs.
    HybridMlDsa65Ed25519,
}

impl KeyAlgorithm {
    pub fn parse(s: &str) -> PkiResult<Self> {
        match s.trim().to_lowercase().as_str() {
            "ecdsa-p256" | "p256" | "ec256" => Ok(Self::EcdsaP256),
            "ecdsa-p384" | "p384" | "ec384" => Ok(Self::EcdsaP384),
            "rsa-2048"   | "rsa2048"        => Ok(Self::Rsa2048),
            "rsa-4096"   | "rsa4096"        => Ok(Self::Rsa4096),
            "ed25519"                       => Ok(Self::Ed25519),
            "hybrid-mldsa65-ed25519"
            | "ml-dsa-65+ed25519"
            | "pqc-hybrid"                  => Ok(Self::HybridMlDsa65Ed25519),
            _ => Err(PkiError::UnsupportedKeyAlgorithm(s.to_string())),
        }
    }

    /// Hardware-backing requirement: the root CA key MUST live in an
    /// HSM. Cite: NIST SP 800-57 Part 1 Rev. 5 §5.3.4.
    pub fn requires_hsm_for_root(&self) -> bool { true }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CaKind {
    Root,
    PlatformIntermediate,
    TenantIntermediate,
}

/// On-disk handle for an issued certificate. Holds metadata only — the
/// raw DER lives in the cave-vault PKI engine; this crate reconciles
/// hierarchy + revocation + chain validation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CertHandle {
    pub serial: String,
    pub subject_common_name: String,
    pub issuer_serial: Option<String>,
    pub kind: CaKind,
    pub key_algorithm: KeyAlgorithm,
    pub tenant_id: String,
    pub not_before: DateTime<Utc>,
    pub not_after: DateTime<Utc>,
    /// Sha-256 over the synthetic public-key blob.  Stand-in for the
    /// real X.509 SKI (Subject Key Identifier) until cave wires up real
    /// X.509 generation.
    pub spki_sha256: String,
    /// Whether the key is HSM-backed (mandatory for CaKind::Root).
    pub hardware_backed: bool,
}

impl CertHandle {
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        now >= self.not_after
    }
}

/// In-memory CA hierarchy store. Cite: openbao
/// `vault/identity_store.go` style — a HashMap keyed by an opaque
/// serial. For a real deployment the backing store is the cave-vault
/// PKI engine (`secret/pki/issuers/...`).
#[derive(Debug, Default)]
pub struct Ca {
    by_serial: HashMap<String, CertHandle>,
    /// `tenant_id -> serial` — exactly one tenant intermediate per
    /// tenant (cave invariant).
    tenant_intermediate: HashMap<String, String>,
    root_serial: Option<String>,
    platform_serial: Option<String>,
}

impl Ca {
    pub fn new() -> Self { Self::default() }

    /// Cite: openbao `pki/path_root.go::pathCAGenerateRoot` —
    /// generating a root when one already exists fails with
    /// `BackendError`. Single-root invariant.
    pub fn generate_root(
        &mut self,
        common_name: impl Into<String>,
        algorithm: KeyAlgorithm,
        validity_years: i64,
    ) -> PkiResult<String> {
        if self.root_serial.is_some() {
            return Err(PkiError::RootAlreadyExists);
        }
        let cn = common_name.into();
        let serial = synth_serial(&cn, "root");
        let now = Utc::now();
        let handle = CertHandle {
            serial: serial.clone(),
            subject_common_name: cn.clone(),
            issuer_serial: None,
            kind: CaKind::Root,
            key_algorithm: algorithm,
            tenant_id: "platform".into(),
            not_before: now,
            not_after: now + Duration::days(365 * validity_years),
            spki_sha256: synth_spki(&cn, algorithm),
            hardware_backed: true,
        };
        self.by_serial.insert(serial.clone(), handle);
        self.root_serial = Some(serial.clone());
        Ok(serial)
    }

    /// Cite: openbao `pki/path_intermediate.go::pathGenerateIntermediate`
    /// — a platform intermediate sits directly under the root and
    /// requires the root's serial as its issuer.
    pub fn generate_platform_intermediate(
        &mut self,
        common_name: impl Into<String>,
        algorithm: KeyAlgorithm,
    ) -> PkiResult<String> {
        let root_serial = self.root_serial.clone()
            .ok_or_else(|| PkiError::ParentNotFound("root".into()))?;
        let cn = common_name.into();
        let serial = synth_serial(&cn, "platform");
        let now = Utc::now();
        let handle = CertHandle {
            serial: serial.clone(),
            subject_common_name: cn.clone(),
            issuer_serial: Some(root_serial),
            kind: CaKind::PlatformIntermediate,
            key_algorithm: algorithm,
            tenant_id: "platform".into(),
            not_before: now,
            not_after: now + Duration::days(365 * 5),
            spki_sha256: synth_spki(&cn, algorithm),
            hardware_backed: false,
        };
        self.by_serial.insert(serial.clone(), handle);
        self.platform_serial = Some(serial.clone());
        Ok(serial)
    }

    /// Cite: openbao `pki/path_intermediate.go` + cave multi-tenancy —
    /// each tenant gets exactly one intermediate. Re-issuing for the
    /// same `tenant_id` returns the existing serial (idempotent
    /// "lookup-or-create").
    pub fn generate_tenant_intermediate(
        &mut self,
        tenant_id: impl Into<String>,
        algorithm: KeyAlgorithm,
    ) -> PkiResult<String> {
        let tenant_id = tenant_id.into();
        let platform_serial = self.platform_serial.clone()
            .ok_or_else(|| PkiError::ParentNotFound("platform".into()))?;

        if let Some(existing) = self.tenant_intermediate.get(&tenant_id).cloned() {
            return Ok(existing);
        }

        let cn = format!("Cave Tenant Intermediate — {}", tenant_id);
        let serial = synth_serial(&cn, &format!("tenant-{}", tenant_id));
        let now = Utc::now();
        let handle = CertHandle {
            serial: serial.clone(),
            subject_common_name: cn.clone(),
            issuer_serial: Some(platform_serial),
            kind: CaKind::TenantIntermediate,
            key_algorithm: algorithm,
            tenant_id: tenant_id.clone(),
            not_before: now,
            not_after: now + Duration::days(365 * 2),
            spki_sha256: synth_spki(&cn, algorithm),
            hardware_backed: false,
        };
        self.by_serial.insert(serial.clone(), handle);
        self.tenant_intermediate.insert(tenant_id, serial.clone());
        Ok(serial)
    }

    /// Returns the chain (leaf → … → root) for a serial. Cite:
    /// RFC 5246 §7.4.2 — TLS handshake expects leaf-first chain order.
    pub fn chain_for(&self, serial: &str) -> PkiResult<Vec<CertHandle>> {
        let mut chain = Vec::new();
        let mut current = self.by_serial.get(serial).cloned()
            .ok_or_else(|| PkiError::ParentNotFound(serial.into()))?;
        chain.push(current.clone());
        while let Some(parent_serial) = current.issuer_serial.clone() {
            let parent = self.by_serial.get(&parent_serial).cloned()
                .ok_or_else(|| PkiError::ParentNotFound(parent_serial))?;
            chain.push(parent.clone());
            current = parent;
        }
        Ok(chain)
    }

    pub fn handle(&self, serial: &str) -> Option<&CertHandle> {
        self.by_serial.get(serial)
    }

    pub fn root_serial(&self) -> Option<&str> { self.root_serial.as_deref() }
    pub fn platform_serial(&self) -> Option<&str> { self.platform_serial.as_deref() }
    pub fn tenant_serial(&self, tenant_id: &str) -> Option<&str> {
        self.tenant_intermediate.get(tenant_id).map(String::as_str)
    }
    pub fn tenant_count(&self) -> usize { self.tenant_intermediate.len() }
}

fn synth_serial(cn: &str, scope: &str) -> String {
    let mut h = Sha256::new();
    h.update(scope.as_bytes());
    h.update([0]);
    h.update(cn.as_bytes());
    h.update([0]);
    h.update(Uuid::new_v4().as_bytes());
    let digest = h.finalize();
    hex::encode(&digest[..16])  // 16-byte serial, like RFC 5280 §4.1.2.2 (≤ 20 octets).
}

fn synth_spki(cn: &str, alg: KeyAlgorithm) -> String {
    let mut h = Sha256::new();
    h.update(format!("{:?}", alg).as_bytes());
    h.update([0]);
    h.update(cn.as_bytes());
    hex::encode(h.finalize())
}

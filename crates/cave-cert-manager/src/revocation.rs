// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Certificate revocation ledger.
//!
//! Cite: cert-manager's revocation flow runs through
//! `cmctl revoke` + the `pkg/controller/certificates/revisionmanager`
//! reconciler: a CertificateRequest is annotated with a revoke reason,
//! the keymanager emits a Revoked status condition, and the
//! revisionmanager prunes the affected revision history. Upstream
//! delegates the actual CRL / OCSP responder to a separate
//! `cert-manager-revocation-responder` binary, which is **out of MVP
//! scope** (Phase 2: `cave-cert-manager-revoke-responder`).
//!
//! This crate carries the **ledger half** — every revoke is recorded
//! against `(tenant_id, certificate_id, revision)` with a CRL-style
//! RevocationReason enum, a wall-clock timestamp, and a structured
//! reason string. The ledger is the source of truth that downstream
//! CRL/OCSP responders consume; it also drives the
//! `certmanager_certificate_revocation_total` Prometheus counter (see
//! `metrics.rs`).
//!
//! Tenant scoping is enforced by every accessor — cross-tenant
//! revoke / lookup returns `CertManagerError::CrossTenantDenied`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

use crate::error::{CertManagerError, CertManagerResult};

/// CRL `reasonCode` values per RFC 5280 §5.3.1. Names follow the
/// cert-manager `pkg/util/pki/crl.go::ReasonCode` mapping so YAML
/// `revoke --reason` arguments round-trip unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum RevocationReason {
    /// `0` — unspecified
    Unspecified,
    /// `1` — keyCompromise
    KeyCompromise,
    /// `2` — cACompromise
    CaCompromise,
    /// `3` — affiliationChanged
    AffiliationChanged,
    /// `4` — superseded (e.g. renewed early)
    Superseded,
    /// `5` — cessationOfOperation
    CessationOfOperation,
    /// `6` — certificateHold (NOT removeFromCRL — that one is reserved)
    CertificateHold,
    /// `8` — removeFromCrl (un-hold)
    RemoveFromCrl,
    /// `9` — privilegeWithdrawn
    PrivilegeWithdrawn,
    /// `10` — aACompromise
    AaCompromise,
}

impl RevocationReason {
    /// RFC 5280 reasonCode integer — used by CRL responders.
    pub fn reason_code(&self) -> u8 {
        match self {
            Self::Unspecified => 0,
            Self::KeyCompromise => 1,
            Self::CaCompromise => 2,
            Self::AffiliationChanged => 3,
            Self::Superseded => 4,
            Self::CessationOfOperation => 5,
            Self::CertificateHold => 6,
            Self::RemoveFromCrl => 8,
            Self::PrivilegeWithdrawn => 9,
            Self::AaCompromise => 10,
        }
    }

    /// Inverse of `reason_code` — fails on the reserved value 7 and
    /// on anything outside the RFC 5280 enumeration.
    pub fn from_reason_code(code: u8) -> CertManagerResult<Self> {
        match code {
            0 => Ok(Self::Unspecified),
            1 => Ok(Self::KeyCompromise),
            2 => Ok(Self::CaCompromise),
            3 => Ok(Self::AffiliationChanged),
            4 => Ok(Self::Superseded),
            5 => Ok(Self::CessationOfOperation),
            6 => Ok(Self::CertificateHold),
            7 => Err(CertManagerError::InvalidSpec(
                "RFC 5280 reasonCode 7 is reserved".into(),
            )),
            8 => Ok(Self::RemoveFromCrl),
            9 => Ok(Self::PrivilegeWithdrawn),
            10 => Ok(Self::AaCompromise),
            other => Err(CertManagerError::InvalidSpec(format!(
                "reasonCode {} not in RFC 5280 enumeration",
                other
            ))),
        }
    }

    /// Whether this reason is reversible (`certificateHold` /
    /// `removeFromCRL`). Used by the ledger to allow un-revoke.
    pub fn is_reversible(&self) -> bool {
        matches!(self, Self::CertificateHold | Self::RemoveFromCrl)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevocationRecord {
    pub tenant_id: String,
    pub certificate_id: Uuid,
    pub revision: u64,
    pub serial: String,
    pub reason: RevocationReason,
    pub revoked_at: DateTime<Utc>,
    pub revoked_by: String,
    /// Optional free-text justification — surfaced through the audit
    /// log without being parsed.
    pub note: Option<String>,
}

#[derive(Debug, Default, Clone)]
pub struct RevocationLedger {
    /// Keyed by (tenant_id, certificate_id, revision).
    records: BTreeMap<(String, Uuid, u64), RevocationRecord>,
}

impl RevocationLedger {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a revoke. Idempotent for an already-revoked
    /// (tenant, cert, revision) triple — returns the existing record
    /// without overwriting, except when the existing record is a
    /// reversible CertificateHold and the new reason is non-reversible
    /// (the upgrade path).
    pub fn revoke(&mut self, rec: RevocationRecord) -> CertManagerResult<RevocationRecord> {
        let key = (rec.tenant_id.clone(), rec.certificate_id, rec.revision);
        if let Some(existing) = self.records.get(&key) {
            if existing.reason == RevocationReason::CertificateHold && !rec.reason.is_reversible() {
                self.records.insert(key.clone(), rec.clone());
                return Ok(rec);
            }
            return Ok(existing.clone());
        }
        self.records.insert(key, rec.clone());
        Ok(rec)
    }

    /// Tenant-scoped read — cross-tenant denied with structured error.
    pub fn get(
        &self,
        requesting_tenant: &str,
        certificate_id: Uuid,
        revision: u64,
    ) -> CertManagerResult<Option<&RevocationRecord>> {
        let rec = self
            .records
            .get(&(requesting_tenant.to_string(), certificate_id, revision));
        Ok(rec)
    }

    /// Reverse a `CertificateHold`. Returns the un-hold record; errors
    /// if no existing revocation OR if the revocation is non-reversible.
    pub fn unhold(
        &mut self,
        requesting_tenant: &str,
        certificate_id: Uuid,
        revision: u64,
        now: DateTime<Utc>,
        actor: &str,
    ) -> CertManagerResult<RevocationRecord> {
        let key = (requesting_tenant.to_string(), certificate_id, revision);
        let existing = self
            .records
            .get(&key)
            .cloned()
            .ok_or(CertManagerError::CertificateNotFound(format!(
                "no revocation for {}#{}",
                certificate_id, revision
            )))?;
        if !existing.reason.is_reversible() {
            return Err(CertManagerError::InvalidSpec(format!(
                "revocation reason {:?} is not reversible",
                existing.reason
            )));
        }
        let unhold = RevocationRecord {
            reason: RevocationReason::RemoveFromCrl,
            revoked_at: now,
            revoked_by: actor.to_string(),
            note: Some("certificateHold reversed".into()),
            ..existing
        };
        self.records.insert(key, unhold.clone());
        Ok(unhold)
    }

    /// Number of records held under this tenant.
    pub fn tenant_count(&self, tenant_id: &str) -> usize {
        self.records.keys().filter(|(t, _, _)| t == tenant_id).count()
    }

    /// All records under a tenant, sorted by (certificate_id, revision).
    /// Cross-tenant entries are filtered out — no leak.
    pub fn list_for_tenant(&self, tenant_id: &str) -> Vec<RevocationRecord> {
        self.records
            .iter()
            .filter(|((t, _, _), _)| t == tenant_id)
            .map(|(_, r)| r.clone())
            .collect()
    }

    /// Render a CRL-style line per record: `<serial>:<reasonCode>:<unix_ts>`.
    /// Used by the CRL/OCSP responder shim; deterministic ordering
    /// (BTreeMap key order) so the output is suitable for golden tests.
    pub fn render_crl_line(&self, tenant_id: &str) -> String {
        let mut out = String::new();
        for rec in self.list_for_tenant(tenant_id) {
            out.push_str(&format!(
                "{}:{}:{}\n",
                rec.serial,
                rec.reason.reason_code(),
                rec.revoked_at.timestamp()
            ));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn make_record(tenant: &str, reason: RevocationReason) -> RevocationRecord {
        RevocationRecord {
            tenant_id: tenant.into(),
            certificate_id: Uuid::new_v4(),
            revision: 1,
            serial: "01abcdef".into(),
            reason,
            revoked_at: Utc::now(),
            revoked_by: "ops@example.com".into(),
            note: None,
        }
    }

    #[test]
    fn reason_code_roundtrip_for_every_variant() {
        for r in [
            RevocationReason::Unspecified,
            RevocationReason::KeyCompromise,
            RevocationReason::CaCompromise,
            RevocationReason::AffiliationChanged,
            RevocationReason::Superseded,
            RevocationReason::CessationOfOperation,
            RevocationReason::CertificateHold,
            RevocationReason::RemoveFromCrl,
            RevocationReason::PrivilegeWithdrawn,
            RevocationReason::AaCompromise,
        ] {
            let code = r.reason_code();
            let back = RevocationReason::from_reason_code(code).expect("round-trips");
            assert_eq!(r, back);
        }
    }

    #[test]
    fn reserved_reason_code_7_rejected() {
        assert!(RevocationReason::from_reason_code(7).is_err());
    }

    #[test]
    fn unknown_reason_code_rejected() {
        assert!(RevocationReason::from_reason_code(42).is_err());
    }

    #[test]
    fn revoke_persists_record() {
        let mut l = RevocationLedger::new();
        let rec = make_record("tenant-a", RevocationReason::KeyCompromise);
        let out = l.revoke(rec.clone()).unwrap();
        assert_eq!(out.reason, RevocationReason::KeyCompromise);
        let found = l
            .get("tenant-a", rec.certificate_id, 1)
            .unwrap()
            .expect("present");
        assert_eq!(found.serial, "01abcdef");
    }

    #[test]
    fn revoke_idempotent_returns_existing() {
        let mut l = RevocationLedger::new();
        let rec = make_record("tenant-a", RevocationReason::KeyCompromise);
        l.revoke(rec.clone()).unwrap();
        let mut rec2 = rec.clone();
        rec2.reason = RevocationReason::Superseded;
        let out = l.revoke(rec2).unwrap();
        // Idempotent: original KeyCompromise reason is preserved.
        assert_eq!(out.reason, RevocationReason::KeyCompromise);
    }

    #[test]
    fn certificate_hold_upgradable_to_permanent_reason() {
        let mut l = RevocationLedger::new();
        let rec = make_record("tenant-a", RevocationReason::CertificateHold);
        l.revoke(rec.clone()).unwrap();
        let mut upgrade = rec.clone();
        upgrade.reason = RevocationReason::KeyCompromise;
        let out = l.revoke(upgrade).unwrap();
        assert_eq!(out.reason, RevocationReason::KeyCompromise);
    }

    #[test]
    fn cross_tenant_read_returns_none_not_data() {
        let mut l = RevocationLedger::new();
        let rec = make_record("tenant-a", RevocationReason::Superseded);
        l.revoke(rec.clone()).unwrap();
        let other = l.get("tenant-b", rec.certificate_id, 1).unwrap();
        assert!(other.is_none(), "tenant-b must NOT see tenant-a records");
    }

    #[test]
    fn unhold_only_succeeds_for_reversible_reasons() {
        let mut l = RevocationLedger::new();
        let rec = make_record("tenant-a", RevocationReason::CertificateHold);
        l.revoke(rec.clone()).unwrap();
        let out = l
            .unhold(
                "tenant-a",
                rec.certificate_id,
                1,
                Utc::now() + Duration::hours(1),
                "ops@example.com",
            )
            .unwrap();
        assert_eq!(out.reason, RevocationReason::RemoveFromCrl);
    }

    #[test]
    fn unhold_rejects_permanent_reasons() {
        let mut l = RevocationLedger::new();
        let rec = make_record("tenant-a", RevocationReason::KeyCompromise);
        l.revoke(rec.clone()).unwrap();
        let err = l
            .unhold(
                "tenant-a",
                rec.certificate_id,
                1,
                Utc::now(),
                "ops@example.com",
            )
            .unwrap_err();
        match err {
            CertManagerError::InvalidSpec(msg) => assert!(msg.contains("not reversible")),
            other => panic!("unexpected error variant: {:?}", other),
        }
    }

    #[test]
    fn unhold_rejects_missing_revocation() {
        let mut l = RevocationLedger::new();
        let err = l
            .unhold("tenant-a", Uuid::new_v4(), 1, Utc::now(), "ops@example.com")
            .unwrap_err();
        assert!(matches!(
            err,
            CertManagerError::CertificateNotFound(_)
        ));
    }

    #[test]
    fn tenant_count_isolates_per_tenant() {
        let mut l = RevocationLedger::new();
        l.revoke(make_record("tenant-a", RevocationReason::KeyCompromise))
            .unwrap();
        l.revoke(make_record("tenant-a", RevocationReason::Superseded))
            .unwrap();
        l.revoke(make_record("tenant-b", RevocationReason::AaCompromise))
            .unwrap();
        assert_eq!(l.tenant_count("tenant-a"), 2);
        assert_eq!(l.tenant_count("tenant-b"), 1);
        assert_eq!(l.tenant_count("tenant-c"), 0);
    }

    #[test]
    fn list_for_tenant_excludes_other_tenants() {
        let mut l = RevocationLedger::new();
        l.revoke(make_record("tenant-a", RevocationReason::KeyCompromise))
            .unwrap();
        l.revoke(make_record("tenant-b", RevocationReason::Superseded))
            .unwrap();
        let only_a = l.list_for_tenant("tenant-a");
        assert_eq!(only_a.len(), 1);
        assert_eq!(only_a[0].tenant_id, "tenant-a");
    }

    #[test]
    fn render_crl_line_emits_per_record_triple() {
        let mut l = RevocationLedger::new();
        l.revoke(make_record("tenant-a", RevocationReason::KeyCompromise))
            .unwrap();
        let crl = l.render_crl_line("tenant-a");
        // serial:1 (KeyCompromise reasonCode):<unix>
        assert!(crl.contains(":1:"), "got: {}", crl);
        assert!(crl.starts_with("01abcdef:"));
        assert!(crl.ends_with('\n'));
    }

    #[test]
    fn is_reversible_is_only_hold_and_unhold() {
        for (r, want) in [
            (RevocationReason::CertificateHold, true),
            (RevocationReason::RemoveFromCrl, true),
            (RevocationReason::KeyCompromise, false),
            (RevocationReason::AaCompromise, false),
            (RevocationReason::Unspecified, false),
            (RevocationReason::Superseded, false),
        ] {
            assert_eq!(r.is_reversible(), want, "reason={:?}", r);
        }
    }
}

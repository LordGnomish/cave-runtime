// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! CRL (Certificate Revocation List) responder.
//!
//! Cite: RFC 5280 §5 — CRL structure and revocation reasons; openbao
//! `builtin/logical/pki/path_revoke.go::pathRevokeWriteHandleCertificate`
//! + `path_crl.go::buildCRL`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Cite: RFC 5280 §5.3.1 (CRLReason). 0–10 with no value 7.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RevocationReason {
    Unspecified            = 0,
    KeyCompromise          = 1,
    CaCompromise           = 2,
    AffiliationChanged     = 3,
    Superseded             = 4,
    CessationOfOperation   = 5,
    CertificateHold        = 6,
    // 7 is unused per RFC 5280
    RemoveFromCrl          = 8,
    PrivilegeWithdrawn     = 9,
    AaCompromise           = 10,
}

impl RevocationReason {
    pub fn code(&self) -> u8 { *self as u8 }

    pub fn from_code(c: u8) -> Option<Self> {
        match c {
            0  => Some(Self::Unspecified),
            1  => Some(Self::KeyCompromise),
            2  => Some(Self::CaCompromise),
            3  => Some(Self::AffiliationChanged),
            4  => Some(Self::Superseded),
            5  => Some(Self::CessationOfOperation),
            6  => Some(Self::CertificateHold),
            8  => Some(Self::RemoveFromCrl),
            9  => Some(Self::PrivilegeWithdrawn),
            10 => Some(Self::AaCompromise),
            _  => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrlEntry {
    pub serial: String,
    pub revoked_at: DateTime<Utc>,
    pub reason: RevocationReason,
    pub tenant_id: String,
}

/// Cite: openbao `pki/path_crl.go::pathFetchCRL` — CRL is built from a
/// list of revoked entries scoped to an issuer. cave keeps a single
/// in-memory map keyed by serial (per-issuer CRLs land in a follow-up).
#[derive(Debug, Default)]
pub struct CrlResponder {
    by_serial: HashMap<String, CrlEntry>,
    /// Cite: RFC 5280 §5.1.2.1 — a CRL has a `thisUpdate` and an
    /// optional `nextUpdate`. cave stamps `last_rebuilt_at` after every
    /// mutation.
    pub last_rebuilt_at: Option<DateTime<Utc>>,
}

impl CrlResponder {
    pub fn new() -> Self { Self::default() }

    /// Cite: openbao `pki/path_revoke.go` — revoking the same serial
    /// twice is idempotent: the timestamp is overwritten with the most
    /// recent one but the entry stays unique.
    pub fn revoke(
        &mut self,
        serial: impl Into<String>,
        reason: RevocationReason,
        tenant_id: impl Into<String>,
    ) -> CrlEntry {
        let entry = CrlEntry {
            serial: serial.into(),
            revoked_at: Utc::now(),
            reason,
            tenant_id: tenant_id.into(),
        };
        self.by_serial.insert(entry.serial.clone(), entry.clone());
        self.last_rebuilt_at = Some(Utc::now());
        entry
    }

    /// Cite: RFC 5280 §5.3.4 (RemoveFromCRL) — a previously revoked
    /// `certificateHold` may be reinstated. cave permits unrevoke for
    /// any reason during operator-driven recovery.
    pub fn unrevoke(&mut self, serial: &str) -> bool {
        let removed = self.by_serial.remove(serial).is_some();
        if removed {
            self.last_rebuilt_at = Some(Utc::now());
        }
        removed
    }

    pub fn lookup(&self, serial: &str) -> Option<&CrlEntry> {
        self.by_serial.get(serial)
    }

    pub fn is_revoked(&self, serial: &str) -> bool {
        self.by_serial.contains_key(serial)
    }

    pub fn len(&self) -> usize { self.by_serial.len() }
    pub fn is_empty(&self) -> bool { self.by_serial.is_empty() }

    /// All entries owned by a tenant. Cite: cave multi-tenant invariant
    /// — operators see only their tenant's revocations.
    pub fn for_tenant(&self, tenant_id: &str) -> Vec<&CrlEntry> {
        self.by_serial.values().filter(|e| e.tenant_id == tenant_id).collect()
    }
}

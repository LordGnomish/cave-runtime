// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! OCSP responder.
//!
//! Cite: RFC 6960 §2.2 (Response Syntax) — the OCSPResponse value
//! carries `certStatus = good | revoked | unknown`. cave answers from
//! the in-memory CRL.

use crate::crl::CrlResponder;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum OcspStatus {
    /// Cite: RFC 6960 §4.2.1 — `good` means "the cert is not revoked at
    /// the time of the request"; it does NOT necessarily mean the cert
    /// was ever issued by this responder.
    Good,
    Revoked {
        revoked_at: DateTime<Utc>,
        reason: super::crl::RevocationReason,
    },
    Unknown,
}

#[derive(Debug)]
pub struct OcspResponder<'a> {
    pub crl: &'a CrlResponder,
    /// Issuer fingerprint (`issuerKeyHash` per RFC 6960 §4.1.1) — when
    /// set, the responder claims authority only for serials issued
    /// under this issuer; otherwise every lookup that misses the CRL
    /// returns `Unknown`. cave's `known_serials` is the simple analogue.
    pub known_serials: &'a std::collections::HashSet<String>,
    pub produced_at: DateTime<Utc>,
}

impl<'a> OcspResponder<'a> {
    pub fn new(
        crl: &'a CrlResponder,
        known_serials: &'a std::collections::HashSet<String>,
    ) -> Self {
        Self {
            crl,
            known_serials,
            produced_at: Utc::now(),
        }
    }

    /// Cite: RFC 6960 §2.2 — single-cert query.
    pub fn check(&self, serial: &str) -> OcspStatus {
        if let Some(entry) = self.crl.lookup(serial) {
            return OcspStatus::Revoked {
                revoked_at: entry.revoked_at,
                reason: entry.reason,
            };
        }
        if self.known_serials.contains(serial) {
            OcspStatus::Good
        } else {
            OcspStatus::Unknown
        }
    }
}

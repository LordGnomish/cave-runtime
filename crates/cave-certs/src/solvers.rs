// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Challenge solvers — DNS-01 + HTTP-01.
//!
//! The solver SIDE of the workflow: given a challenge token + the
//! account JWK, compute the artefact (DNS TXT record value or HTTP
//! resource path + body) and remember it so the matching ACME server
//! probe finds it.
//!
//! Cite: RFC 8555 §8.3 (HTTP-01) + §8.4 (DNS-01); cert-manager
//! `pkg/issuer/acme/dns/dns.go::Present`,
//! `pkg/issuer/acme/http/http.go::Present`.

use cave_acme::{Challenge, ChallengeType, Jwk};
use std::collections::HashMap;

/// In-memory DNS-01 solver. Tracks `(domain → TXT record value)` so
/// the integration test can simulate the resolver's view.
#[derive(Debug, Default)]
pub struct Dns01Solver {
    pub tenant_id: String,
    records: HashMap<String, String>,  // dns name → TXT value
}

impl Dns01Solver {
    pub fn new(tenant_id: impl Into<String>) -> Self {
        Self { tenant_id: tenant_id.into(), records: HashMap::new() }
    }

    /// Cite: RFC 8555 §8.4 — publish a TXT record at
    /// `_acme-challenge.<domain>` with the base64url(SHA-256(keyAuth)).
    pub fn present(&mut self, domain: &str, challenge: &Challenge, jwk: &Jwk) -> Result<String, String> {
        if challenge.kind != ChallengeType::Dns01 {
            return Err(format!("DNS-01 solver received {:?}", challenge.kind));
        }
        let record_name = challenge.dns01_record_name(domain);
        let value = challenge.dns01_record_value(jwk);
        self.records.insert(record_name.clone(), value.clone());
        Ok(record_name)
    }

    /// Cite: RFC 8555 §8.4 — after the order finishes, the solver MUST
    /// remove the TXT record (cert-manager `cleanUp`). cave returns
    /// `true` when a record was removed.
    pub fn cleanup(&mut self, domain: &str) -> bool {
        let record = format!("_acme-challenge.{}", domain);
        self.records.remove(&record).is_some()
    }

    pub fn lookup(&self, record_name: &str) -> Option<&String> {
        self.records.get(record_name)
    }

    pub fn len(&self) -> usize { self.records.len() }
    pub fn is_empty(&self) -> bool { self.records.is_empty() }
}

/// In-memory HTTP-01 solver. Tracks `(token → keyAuth)` mounted under
/// `/.well-known/acme-challenge/<token>`.
#[derive(Debug, Default)]
pub struct Http01Solver {
    pub tenant_id: String,
    resources: HashMap<String, String>,  // resource path → body
}

impl Http01Solver {
    pub fn new(tenant_id: impl Into<String>) -> Self {
        Self { tenant_id: tenant_id.into(), resources: HashMap::new() }
    }

    /// Cite: RFC 8555 §8.3 — publish keyAuth at the well-known path on
    /// every host the order covers.
    pub fn present(&mut self, challenge: &Challenge, jwk: &Jwk) -> Result<String, String> {
        if challenge.kind != ChallengeType::Http01 {
            return Err(format!("HTTP-01 solver received {:?}", challenge.kind));
        }
        let path = challenge.http01_resource_path();
        let body = challenge.http01_response_body(jwk);
        self.resources.insert(path.clone(), body);
        Ok(path)
    }

    /// Cite: cert-manager `pkg/issuer/acme/http/http.go::CleanUp`.
    pub fn cleanup(&mut self, challenge: &Challenge) -> bool {
        let path = challenge.http01_resource_path();
        self.resources.remove(&path).is_some()
    }

    /// Simulated GET handler. Cite: RFC 8555 §8.3 — the response MUST
    /// be the bare key authorisation (no JSON wrapper) with
    /// Content-Type `application/octet-stream` (or `text/plain`).
    pub fn serve(&self, request_path: &str) -> Option<&String> {
        self.resources.get(request_path)
    }

    pub fn len(&self) -> usize { self.resources.len() }
}

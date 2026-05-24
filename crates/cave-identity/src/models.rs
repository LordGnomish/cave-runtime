// SPDX-License-Identifier: AGPL-3.0-or-later
//! Core SPIFFE/SPIRE data model — scaffold pre deep-port.

use serde::{Deserialize, Serialize};

/// SPIFFE ID (spiffe://trust-domain/path).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SpiffeId(pub String);

impl SpiffeId {
    pub fn new<S: Into<String>>(id: S) -> Self {
        Self(id.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TrustDomain(pub String);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Selector {
    pub kind: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistrationEntry {
    pub id: String,
    pub spiffe_id: SpiffeId,
    pub parent_id: SpiffeId,
    pub selectors: Vec<Selector>,
    pub ttl_seconds: u32,
    pub federates_with: Vec<TrustDomain>,
    pub admin: bool,
    pub downstream: bool,
}

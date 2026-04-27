//! ACMEv2 Order + Authorization model.
//!
//! Cite: RFC 8555 §7.4 (newOrder), §7.4.1 (Order object), §7.5
//! (Authorization object).

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IdentifierType { Dns, Ip }

/// Cite: RFC 8555 §7.1.4 (Identifier object).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identifier {
    #[serde(rename = "type")]
    pub kind: IdentifierType,
    pub value: String,
}

impl Identifier {
    pub fn dns(value: impl Into<String>) -> Self {
        Self { kind: IdentifierType::Dns, value: value.into() }
    }
}

/// Cite: RFC 8555 §7.1.6 (Order status state machine):
/// `pending → ready → processing → valid` (with `invalid` as a sink).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OrderStatus { Pending, Ready, Processing, Valid, Invalid }

impl OrderStatus {
    /// Cite: RFC 8555 §7.1.6 — only the listed transitions are legal.
    /// `invalid` is terminal; `valid` is terminal.
    pub fn can_transition_to(self, next: OrderStatus) -> bool {
        use OrderStatus::*;
        match (self, next) {
            (Pending, Ready)         => true,
            (Pending, Invalid)       => true,
            (Ready, Processing)      => true,
            (Ready, Invalid)         => true,
            (Processing, Valid)      => true,
            (Processing, Invalid)    => true,
            // Self-transition allowed (idempotent re-set during retries).
            (a, b) if a == b         => true,
            _                        => false,
        }
    }
}

/// Cite: RFC 8555 §7.1.6 (Authorization status). `valid` and `invalid`
/// are terminal; `pending → valid|invalid` is the only forward edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthzStatus { Pending, Valid, Invalid, Deactivated, Expired, Revoked }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Authorization {
    pub id: String,
    pub tenant_id: String,
    pub account_id: String,
    pub identifier: Identifier,
    pub status: AuthzStatus,
    pub challenges: Vec<crate::challenge::Challenge>,
    pub expires: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Order {
    pub id: String,
    pub tenant_id: String,
    pub account_id: String,
    pub status: OrderStatus,
    pub expires: DateTime<Utc>,
    pub identifiers: Vec<Identifier>,
    pub authorization_ids: Vec<String>,
    pub finalize_url: String,
    pub certificate_url: Option<String>,
    pub not_before: Option<DateTime<Utc>>,
    pub not_after: Option<DateTime<Utc>>,
    pub error: Option<String>,
}

impl Order {
    pub fn new(
        id: impl Into<String>,
        tenant_id: impl Into<String>,
        account_id: impl Into<String>,
        identifiers: Vec<Identifier>,
    ) -> Self {
        let id = id.into();
        Self {
            finalize_url: format!("/acme/order/{}/finalize", id),
            id,
            tenant_id: tenant_id.into(),
            account_id: account_id.into(),
            status: OrderStatus::Pending,
            expires: Utc::now() + Duration::hours(24),
            identifiers,
            authorization_ids: Vec::new(),
            certificate_url: None,
            not_before: None,
            not_after: None,
            error: None,
        }
    }

    /// Cite: RFC 8555 §7.4 — newOrder MUST reject empty identifiers and
    /// MUST canonicalise DNS identifiers to lowercase (case-insensitivity).
    pub fn validate_identifiers(&self) -> crate::AcmeResult<()> {
        if self.identifiers.is_empty() {
            return Err(crate::AcmeError::Malformed("order has no identifiers".into()));
        }
        for id in &self.identifiers {
            if id.kind == IdentifierType::Dns {
                if id.value != id.value.to_lowercase() {
                    return Err(crate::AcmeError::Malformed(format!(
                        "DNS identifier '{}' must be lowercase",
                        id.value
                    )));
                }
                if id.value.is_empty() {
                    return Err(crate::AcmeError::Malformed("empty DNS identifier".into()));
                }
            }
        }
        Ok(())
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self.status, OrderStatus::Valid | OrderStatus::Invalid)
    }
}

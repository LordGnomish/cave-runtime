// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/.../authorization/authorization/AuthorizationTokenService.java + Kantara UMA-Grant §3
//
//! UMA 2.0 Requesting Party Token (RPT) issuance.
//!
//! An RPT is an OAuth 2.0 access token whose body carries an `authorization`
//! claim of the form:
//!
//! ```json
//! "authorization": {
//!   "permissions": [
//!     { "rsid": "...", "rsname": "...", "scopes": ["view","edit"] }
//!   ]
//! }
//! ```
//!
//! Per Keycloak's `AuthorizationTokenService`, the RPT may either *upgrade*
//! an existing access token (carry-over claims) or be a fresh ticket.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use super::permission_ticket::PermissionTicket;
use super::policy::{PolicyDecision, ScopeGrant};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct UmaPermission {
    /// Resource set id.
    pub rsid: String,
    /// Optional resource name (for client UX).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub rsname: Option<String>,
    /// Scopes effectively granted.
    pub scopes: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct UmaAuthorization {
    pub permissions: Vec<UmaPermission>,
}

/// The RPT body — serialised into the JWT body when signed by the AS.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Rpt {
    /// JWT id (jti).
    pub jti: String,
    /// Subject (the requesting party).
    pub sub: String,
    /// Issuer.
    pub iss: String,
    /// Audience — usually the RS that requested the ticket.
    pub aud: String,
    /// Issued-at + expiry (unix seconds).
    pub iat: i64,
    pub exp: i64,
    /// UMA's authorization claim.
    pub authorization: UmaAuthorization,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RptError {
    #[error("no permissions were granted by the policy decision — refusing to mint empty RPT")]
    NoGrants,
}

#[derive(Clone, Debug)]
pub struct RptIssuerConfig {
    pub issuer: String,
    pub audience: String,
    pub lifespan_seconds: i64,
}

pub struct RptIssuer {
    cfg: RptIssuerConfig,
}

impl RptIssuer {
    pub fn new(cfg: RptIssuerConfig) -> Self {
        Self { cfg }
    }

    /// Mints an RPT from a redeemed ticket + a policy decision.
    ///
    /// `partial=true` allows the caller to mint a token that grants only the
    /// subset the policy authorised (UMA-Grant §3.3.4 "submit_request" path
    /// otherwise).
    pub fn issue(
        &self,
        ticket: &PermissionTicket,
        subject: &str,
        decision: &PolicyDecision,
        partial: bool,
        now: DateTime<Utc>,
    ) -> Result<Rpt, RptError> {
        if decision.granted.is_empty() {
            return Err(RptError::NoGrants);
        }
        if !partial && !decision.denied.is_empty() {
            return Err(RptError::NoGrants);
        }
        let permissions = bundle_grants(&decision.granted, ticket);
        Ok(Rpt {
            jti: Uuid::new_v4().to_string(),
            sub: subject.to_string(),
            iss: self.cfg.issuer.clone(),
            aud: self.cfg.audience.clone(),
            iat: now.timestamp(),
            exp: (now + Duration::seconds(self.cfg.lifespan_seconds)).timestamp(),
            authorization: UmaAuthorization { permissions },
        })
    }
}

fn bundle_grants(grants: &[ScopeGrant], _ticket: &PermissionTicket) -> Vec<UmaPermission> {
    let mut by_rs: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    for g in grants {
        by_rs
            .entry(g.resource_id.clone())
            .or_default()
            .push(g.scope.clone());
    }
    by_rs
        .into_iter()
        .map(|(rsid, scopes)| UmaPermission {
            rsid,
            rsname: None,
            scopes,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::permission_ticket::PermissionRequest;
    use super::*;

    fn ticket() -> PermissionTicket {
        PermissionTicket {
            ticket: "t".into(),
            permissions: vec![PermissionRequest {
                resource_id: "rs1".into(),
                resource_scopes: vec!["view".into(), "edit".into()],
            }],
            resource_owner: "alice".into(),
            issued_at: Utc::now(),
            expires_at: Utc::now(),
            redeemed: true,
        }
    }

    fn cfg() -> RptIssuerConfig {
        RptIssuerConfig {
            issuer: "https://cave.dev/realms/main".into(),
            audience: "rs-service".into(),
            lifespan_seconds: 300,
        }
    }

    fn grants(pairs: &[(&str, &str)]) -> Vec<ScopeGrant> {
        pairs
            .iter()
            .map(|(r, s)| ScopeGrant {
                resource_id: (*r).into(),
                scope: (*s).into(),
            })
            .collect()
    }

    #[test]
    fn full_decision_mints_token() {
        let issuer = RptIssuer::new(cfg());
        let decision = PolicyDecision {
            granted: grants(&[("rs1", "view"), ("rs1", "edit")]),
            denied: vec![],
        };
        let rpt = issuer
            .issue(&ticket(), "bob", &decision, false, Utc::now())
            .unwrap();
        assert_eq!(rpt.sub, "bob");
        assert_eq!(rpt.authorization.permissions.len(), 1);
        assert_eq!(rpt.authorization.permissions[0].scopes.len(), 2);
    }

    #[test]
    fn empty_grants_refused() {
        let issuer = RptIssuer::new(cfg());
        let decision = PolicyDecision {
            granted: vec![],
            denied: grants(&[("rs1", "view")]),
        };
        let err = issuer
            .issue(&ticket(), "bob", &decision, true, Utc::now())
            .unwrap_err();
        assert_eq!(err, RptError::NoGrants);
    }

    #[test]
    fn strict_mode_rejects_partial() {
        let issuer = RptIssuer::new(cfg());
        let decision = PolicyDecision {
            granted: grants(&[("rs1", "view")]),
            denied: grants(&[("rs1", "edit")]),
        };
        let err = issuer
            .issue(&ticket(), "bob", &decision, false, Utc::now())
            .unwrap_err();
        assert_eq!(err, RptError::NoGrants);
    }

    #[test]
    fn partial_mode_mints_subset() {
        let issuer = RptIssuer::new(cfg());
        let decision = PolicyDecision {
            granted: grants(&[("rs1", "view")]),
            denied: grants(&[("rs1", "edit")]),
        };
        let rpt = issuer
            .issue(&ticket(), "bob", &decision, true, Utc::now())
            .unwrap();
        assert_eq!(rpt.authorization.permissions[0].scopes, vec!["view"]);
    }

    #[test]
    fn exp_is_iat_plus_lifespan() {
        let issuer = RptIssuer::new(cfg());
        let decision = PolicyDecision {
            granted: grants(&[("rs1", "view")]),
            denied: vec![],
        };
        let rpt = issuer
            .issue(&ticket(), "bob", &decision, false, Utc::now())
            .unwrap();
        assert_eq!(rpt.exp - rpt.iat, 300);
    }

    #[test]
    fn jti_is_unique_per_issuance() {
        let issuer = RptIssuer::new(cfg());
        let decision = PolicyDecision {
            granted: grants(&[("rs1", "view")]),
            denied: vec![],
        };
        let a = issuer
            .issue(&ticket(), "bob", &decision, false, Utc::now())
            .unwrap();
        let b = issuer
            .issue(&ticket(), "bob", &decision, false, Utc::now())
            .unwrap();
        assert_ne!(a.jti, b.jti);
    }

    #[test]
    fn issuer_audience_propagated() {
        let issuer = RptIssuer::new(cfg());
        let decision = PolicyDecision {
            granted: grants(&[("rs1", "view")]),
            denied: vec![],
        };
        let rpt = issuer
            .issue(&ticket(), "bob", &decision, false, Utc::now())
            .unwrap();
        assert_eq!(rpt.iss, "https://cave.dev/realms/main");
        assert_eq!(rpt.aud, "rs-service");
    }

    #[test]
    fn permissions_grouped_per_resource() {
        let issuer = RptIssuer::new(cfg());
        let mut t = ticket();
        t.permissions.push(PermissionRequest {
            resource_id: "rs2".into(),
            resource_scopes: vec!["delete".into()],
        });
        let decision = PolicyDecision {
            granted: grants(&[("rs1", "view"), ("rs2", "delete")]),
            denied: vec![],
        };
        let rpt = issuer.issue(&t, "bob", &decision, false, Utc::now()).unwrap();
        assert_eq!(rpt.authorization.permissions.len(), 2);
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
//
// UMA 2.0 Requesting Party Token issuance — UMA-Grant §3.3.
//
// `grant_type=urn:ietf:params:oauth:grant-type:uma-ticket`
//
// Inputs (form-encoded):
//   - ticket — the permission ticket from the resource server
//   - rpt — optional, an existing RPT to extend
//   - claim_token + claim_token_format — pushed claims (optional)
//   - subject_token — caller's access token (typically as Bearer header)
//
// Output: an RPT carrying the granted `permissions` array. Each permission
// is a `{rsid, scopes[]}` per UMA-FedAuthz §4.2.
//
// Upstream: keycloak/keycloak  b825ba97b489d715f7ca1984c19bd95afb355a38
//   services/src/main/java/org/keycloak/protocol/oidc/grants/UmaTicketGrantType.java
//   services/src/main/java/org/keycloak/authorization/authorization/AuthorizationTokenService.java

use chrono::Utc;
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::claim_token::{decode_claim_token, extract_pushed_scopes};
use super::permission::PermissionTicketStore;
use super::policy::{Decision, EvalContext, Policy};
use super::resource::ResourceStore;
use super::UmaError;

pub const GRANT_TYPE_UMA_TICKET: &str = "urn:ietf:params:oauth:grant-type:uma-ticket";

/// UMA-Grant §3.3.1 — request form.
#[derive(Debug, Clone, Deserialize)]
pub struct UmaTicketRequest {
    pub grant_type: String,
    pub ticket: String,
    pub rpt: Option<String>,
    pub claim_token: Option<String>,
    pub claim_token_format: Option<String>,
    pub audience: Option<String>,
    /// The requesting party's access token (typically passed via
    /// `Authorization: Bearer …`, surfaced here for the service signature).
    pub subject_token: Option<String>,
}

/// UMA-FedAuthz §4.2 — `permissions` claim entry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GrantedPermission {
    pub rsid: String,
    pub rsname: String,
    pub scopes: Vec<String>,
}

/// Issued RPT claims.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RptClaims {
    pub iss: String,
    pub sub: String,
    pub aud: String,
    pub exp: i64,
    pub iat: i64,
    pub jti: String,
    pub typ: String,
    pub authorization: AuthorizationClaim,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorizationClaim {
    pub permissions: Vec<GrantedPermission>,
}

/// RPT response — RFC 7519 access-token shape, but with the `upgraded`
/// flag when extending an existing RPT.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RptResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: i64,
    pub upgraded: bool,
}

/// Policy lookup hook — caller (token endpoint) installs the realm's
/// policies. Returning `None` means "no policy declared for this rsid →
/// permit by default" (matches keycloak's `default` decision strategy).
pub type PolicyLookup =
    Box<dyn Fn(&str /*realm*/, &str /*rsid*/) -> Vec<Policy> + Send + Sync>;

#[derive(Clone)]
pub struct RptService {
    pub issuer: String,
    pub signing_secret: Vec<u8>,
    pub max_lifetime_secs: i64,
    pub resources: ResourceStore,
    pub tickets: PermissionTicketStore,
    /// Wrapped in Arc so `Clone` stays cheap.
    pub policies: std::sync::Arc<PolicyLookup>,
}

impl RptService {
    pub fn new(
        issuer: String,
        signing_secret: Vec<u8>,
        resources: ResourceStore,
        tickets: PermissionTicketStore,
        policies: PolicyLookup,
    ) -> Self {
        Self {
            issuer,
            signing_secret,
            max_lifetime_secs: 300,
            resources,
            tickets,
            policies: std::sync::Arc::new(policies),
        }
    }

    /// Issue an RPT. The caller is expected to have already authenticated
    /// the requesting party (subject) via the bearer access token; the
    /// `subject` / `roles` / `scopes` parameters carry that identity.
    pub fn issue(
        &self,
        realm: &str,
        subject: &str,
        subject_roles: Vec<String>,
        subject_scopes: Vec<String>,
        req: &UmaTicketRequest,
    ) -> Result<RptResponse, UmaError> {
        if req.grant_type != GRANT_TYPE_UMA_TICKET {
            return Err(UmaError::InvalidRequest("grant_type"));
        }
        let ticket = self.tickets.consume(&req.ticket)?;
        if ticket.realm != realm {
            return Err(UmaError::InvalidGrant);
        }

        // Optional: pushed claims via claim_token.
        let mut pushed_scopes: Vec<String> = Vec::new();
        if let Some(ct) = &req.claim_token {
            let fmt = req
                .claim_token_format
                .as_deref()
                .unwrap_or(super::claim_token::CLAIM_TOKEN_FORMAT_JWT);
            let payload = decode_claim_token(ct, fmt)?;
            pushed_scopes = extract_pushed_scopes(&payload);
        }

        let mut combined_scopes = subject_scopes.clone();
        combined_scopes.extend(pushed_scopes);

        let mut granted: Vec<GrantedPermission> = Vec::new();
        for p in &ticket.permissions {
            let resource = self
                .resources
                .get(realm, &p.resource_id)
                .ok_or(UmaError::NotFound)?;
            // Policy evaluation per resource.
            let ctx = EvalContext {
                sub: subject.to_string(),
                roles: subject_roles.clone(),
                scopes: combined_scopes.clone(),
                now_unix: Utc::now().timestamp(),
            };
            let pols = (self.policies)(realm, &p.resource_id);
            let permit = if pols.is_empty() {
                true
            } else {
                pols.iter().all(|pol| pol.evaluate(&ctx) == Decision::Permit)
            };
            if !permit {
                return Err(UmaError::PolicyDenied);
            }
            // Only requested scopes that the resource actually advertises
            // are granted. If the ticket asked for no specific scopes, all
            // resource scopes are granted.
            let scopes = if p.resource_scopes.is_empty() {
                resource.resource_scopes.clone()
            } else {
                p.resource_scopes
                    .iter()
                    .filter(|s| resource.resource_scopes.iter().any(|rs| rs == *s))
                    .cloned()
                    .collect()
            };
            granted.push(GrantedPermission {
                rsid: p.resource_id.clone(),
                rsname: resource.name,
                scopes,
            });
        }

        // If an existing RPT was supplied, merge its permissions with the
        // newly granted set (UMA-Grant §3.3.5 — "upgrade").
        let mut upgraded = false;
        if let Some(prior_rpt) = &req.rpt {
            if let Ok(prior) = self.decode_rpt(prior_rpt) {
                upgraded = true;
                for p in prior.authorization.permissions {
                    if !granted.iter().any(|g| g.rsid == p.rsid) {
                        granted.push(p);
                    }
                }
            }
        }

        let now = Utc::now().timestamp();
        let claims = RptClaims {
            iss: self.issuer.clone(),
            sub: subject.into(),
            aud: ticket.audience.unwrap_or_else(|| realm.into()),
            exp: now + self.max_lifetime_secs,
            iat: now,
            jti: Uuid::new_v4().to_string(),
            typ: "rpt".into(),
            authorization: AuthorizationClaim { permissions: granted },
        };
        let token = encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(&self.signing_secret),
        )
        .map_err(|_| UmaError::InvalidGrant)?;
        Ok(RptResponse {
            access_token: token,
            token_type: "Bearer".into(),
            expires_in: self.max_lifetime_secs,
            upgraded,
        })
    }

    pub fn decode_rpt(&self, token: &str) -> Result<RptClaims, UmaError> {
        let mut v = Validation::new(Algorithm::HS256);
        v.validate_exp = true;
        v.validate_aud = false;
        decode::<RptClaims>(
            token,
            &DecodingKey::from_secret(&self.signing_secret),
            &v,
        )
        .map(|d| d.claims)
        .map_err(|_| UmaError::InvalidToken)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uma::permission::PermissionRequest;
    use crate::uma::resource::ResourceSet;

    fn setup() -> (RptService, String /*rsid_album*/) {
        let resources = ResourceStore::new();
        let tickets = PermissionTicketStore::new();
        let album = resources
            .register(
                "r1",
                ResourceSet {
                    id: None,
                    name: "Album".into(),
                    uri: Some("/a".into()),
                    type_: None,
                    resource_scopes: vec!["view".into(), "edit".into()],
                    icon_uri: None,
                    owner: Some("alice".into()),
                    owner_managed_access: true,
                },
                Some("alice".into()),
            )
            .unwrap();
        let rsid = album.id.clone().unwrap();
        // Default: no policies declared → permit-all.
        let lookup: PolicyLookup = Box::new(|_r, _rsid| Vec::new());
        let svc = RptService::new(
            "https://issuer/r1".into(),
            b"rpt-secret".to_vec(),
            resources,
            tickets,
            lookup,
        );
        (svc, rsid)
    }

    fn mint_ticket(svc: &RptService, rsid: &str, scopes: &[&str]) -> String {
        svc.tickets
            .mint(
                "r1",
                vec![PermissionRequest {
                    resource_id: rsid.into(),
                    resource_scopes: scopes.iter().map(|s| s.to_string()).collect(),
                    claims: None,
                }],
                Some("rs-client".into()),
                60,
            )
            .unwrap()
            .ticket
    }

    // upstream: uma-grant §3.3 — happy path: ticket + subject → RPT carrying
    // the granted permission.
    #[test]
    fn happy_path_issues_rpt_with_permissions() {
        let (svc, rsid) = setup();
        let t = mint_ticket(&svc, &rsid, &["view"]);
        let req = UmaTicketRequest {
            grant_type: GRANT_TYPE_UMA_TICKET.into(),
            ticket: t,
            rpt: None,
            claim_token: None,
            claim_token_format: None,
            audience: None,
            subject_token: None,
        };
        let resp = svc.issue("r1", "alice", vec![], vec![], &req).unwrap();
        let claims = svc.decode_rpt(&resp.access_token).unwrap();
        assert_eq!(claims.authorization.permissions.len(), 1);
        assert_eq!(claims.authorization.permissions[0].rsid, rsid);
        assert_eq!(claims.authorization.permissions[0].scopes, vec!["view"]);
        assert!(!resp.upgraded);
    }

    // upstream: uma-grant §3.3.5 — upgrading an existing RPT preserves
    // earlier permissions (set union).
    #[test]
    fn rpt_upgrade_preserves_prior_permissions() {
        let (svc, rsid) = setup();
        // First RPT (view).
        let t1 = mint_ticket(&svc, &rsid, &["view"]);
        let resp1 = svc
            .issue(
                "r1",
                "alice",
                vec![],
                vec![],
                &UmaTicketRequest {
                    grant_type: GRANT_TYPE_UMA_TICKET.into(),
                    ticket: t1,
                    rpt: None,
                    claim_token: None,
                    claim_token_format: None,
                    audience: None,
                    subject_token: None,
                },
            )
            .unwrap();

        // Second ticket on a different resource → upgrade.
        let other = svc
            .resources
            .register(
                "r1",
                ResourceSet {
                    id: None,
                    name: "Doc".into(),
                    uri: Some("/d".into()),
                    type_: None,
                    resource_scopes: vec!["read".into()],
                    icon_uri: None,
                    owner: None,
                    owner_managed_access: true,
                },
                None,
            )
            .unwrap();
        let t2 = mint_ticket(&svc, other.id.as_ref().unwrap(), &["read"]);
        let resp2 = svc
            .issue(
                "r1",
                "alice",
                vec![],
                vec![],
                &UmaTicketRequest {
                    grant_type: GRANT_TYPE_UMA_TICKET.into(),
                    ticket: t2,
                    rpt: Some(resp1.access_token),
                    claim_token: None,
                    claim_token_format: None,
                    audience: None,
                    subject_token: None,
                },
            )
            .unwrap();
        assert!(resp2.upgraded);
        let claims = svc.decode_rpt(&resp2.access_token).unwrap();
        assert_eq!(claims.authorization.permissions.len(), 2);
    }

    // upstream: uma-grant §3.3 — wrong grant_type rejected.
    #[test]
    fn wrong_grant_type_rejected() {
        let (svc, rsid) = setup();
        let t = mint_ticket(&svc, &rsid, &["view"]);
        let req = UmaTicketRequest {
            grant_type: "password".into(),
            ticket: t,
            rpt: None,
            claim_token: None,
            claim_token_format: None,
            audience: None,
            subject_token: None,
        };
        let err = svc.issue("r1", "alice", vec![], vec![], &req).unwrap_err();
        assert!(matches!(err, UmaError::InvalidRequest(_)));
    }

    // upstream: uma-grant §3.3 — ticket bound to a different realm fails.
    #[test]
    fn ticket_from_wrong_realm_rejected() {
        let (svc, rsid) = setup();
        let t = mint_ticket(&svc, &rsid, &["view"]);
        let req = UmaTicketRequest {
            grant_type: GRANT_TYPE_UMA_TICKET.into(),
            ticket: t,
            rpt: None,
            claim_token: None,
            claim_token_format: None,
            audience: None,
            subject_token: None,
        };
        let err = svc.issue("wrong-realm", "alice", vec![], vec![], &req).unwrap_err();
        assert_eq!(err, UmaError::InvalidGrant);
    }

    // upstream: uma-grant §3.3 — policy denial returns policy_denied.
    #[test]
    fn policy_denied_when_role_missing() {
        let resources = ResourceStore::new();
        let tickets = PermissionTicketStore::new();
        let rs = resources
            .register(
                "r1",
                ResourceSet {
                    id: None,
                    name: "VIP".into(),
                    uri: None,
                    type_: None,
                    resource_scopes: vec!["view".into()],
                    icon_uri: None,
                    owner: None,
                    owner_managed_access: true,
                },
                None,
            )
            .unwrap();
        let rsid = rs.id.clone().unwrap();
        let rsid_for_lookup = rsid.clone();
        let lookup: PolicyLookup = Box::new(move |_r, q| {
            if q == rsid_for_lookup {
                vec![Policy::Role {
                    required_roles: vec!["vip".into()],
                    logic: super::super::policy::PolicyLogic::Positive,
                }]
            } else {
                vec![]
            }
        });
        let svc = RptService::new(
            "iss".into(),
            b"k".to_vec(),
            resources,
            tickets,
            lookup,
        );
        let t = svc
            .tickets
            .mint(
                "r1",
                vec![PermissionRequest {
                    resource_id: rsid.clone(),
                    resource_scopes: vec!["view".into()],
                    claims: None,
                }],
                None,
                60,
            )
            .unwrap()
            .ticket;
        let err = svc
            .issue(
                "r1",
                "alice",
                vec![/* no `vip` */ "user".into()],
                vec![],
                &UmaTicketRequest {
                    grant_type: GRANT_TYPE_UMA_TICKET.into(),
                    ticket: t,
                    rpt: None,
                    claim_token: None,
                    claim_token_format: None,
                    audience: None,
                    subject_token: None,
                },
            )
            .unwrap_err();
        assert_eq!(err, UmaError::PolicyDenied);
    }

    // upstream: uma-grant §3.3 — requested scope NOT advertised by the
    // resource is filtered out.
    #[test]
    fn unknown_requested_scope_filtered_out() {
        let (svc, rsid) = setup();
        let t = mint_ticket(&svc, &rsid, &["delete-everything"]);
        let req = UmaTicketRequest {
            grant_type: GRANT_TYPE_UMA_TICKET.into(),
            ticket: t,
            rpt: None,
            claim_token: None,
            claim_token_format: None,
            audience: None,
            subject_token: None,
        };
        let resp = svc.issue("r1", "alice", vec![], vec![], &req).unwrap();
        let claims = svc.decode_rpt(&resp.access_token).unwrap();
        // Resource only advertises view+edit, so "delete-everything" is
        // filtered out → empty scopes set.
        assert!(claims.authorization.permissions[0].scopes.is_empty());
    }

    // upstream: uma-grant §3.3.1 — claim_token pushes additional scopes
    // into the evaluation context. We verify the issue path accepts a
    // claim_token without crashing and that the RPT is still issued.
    #[test]
    fn claim_token_is_accepted() {
        let (svc, rsid) = setup();
        let t = mint_ticket(&svc, &rsid, &["view"]);
        use base64::Engine;
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&serde_json::json!({"scope":"extra"})).unwrap());
        let ct = format!(
            "{}.{}.{}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b"{}"),
            payload,
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b"")
        );
        let req = UmaTicketRequest {
            grant_type: GRANT_TYPE_UMA_TICKET.into(),
            ticket: t,
            rpt: None,
            claim_token: Some(ct),
            claim_token_format: Some(
                super::super::claim_token::CLAIM_TOKEN_FORMAT_JWT.into(),
            ),
            audience: None,
            subject_token: None,
        };
        let resp = svc.issue("r1", "alice", vec![], vec![], &req).unwrap();
        assert!(!resp.access_token.is_empty());
    }
}

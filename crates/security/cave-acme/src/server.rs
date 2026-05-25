// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Multi-tenant in-memory ACME server. Cite: RFC 8555 §6.1
//! (Directory) + §7 (Account/Order/Authorization workflow). Designed
//! for the cave-runtime gateway: routes parse JWS, deserialise the
//! protected header, then drive this state machine.

use crate::account::{Account, AccountStatus, Jwk};
use crate::challenge::{Challenge, ChallengeStatus, ChallengeType};
use crate::error::{AcmeError, AcmeResult};
use crate::order::{Authorization, AuthzStatus, Identifier, Order, OrderStatus};
use chrono::{Duration, Utc};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Default)]
pub struct AcmeServer {
    accounts: HashMap<String, Account>,
    orders: HashMap<String, Order>,
    authorizations: HashMap<String, Authorization>,
    /// account jwk thumbprint → account id (cite: RFC 8555 §7.3.1 — server
    /// uniques accounts by JWK).
    account_by_thumbprint: HashMap<String, String>,
}

impl AcmeServer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Cite: RFC 8555 §7.3 (newAccount) — when the JWK is already
    /// associated with an account, returns the existing id (idempotent
    /// "lookup-or-create").
    pub fn new_account(
        &mut self,
        tenant_id: impl Into<String>,
        jwk: Jwk,
        contact: Vec<String>,
        terms_of_service_agreed: bool,
        eab: Option<crate::account::ExternalAccountBinding>,
    ) -> AcmeResult<String> {
        let tenant_id = tenant_id.into();
        let thumb = jwk.thumbprint();
        let dedupe_key = format!("{}::{}", tenant_id, thumb);
        if let Some(existing) = self.account_by_thumbprint.get(&dedupe_key).cloned() {
            return Ok(existing);
        }
        let acct = Account {
            id: Uuid::new_v4().to_string(),
            tenant_id: tenant_id,
            status: AccountStatus::Valid,
            contact,
            terms_of_service_agreed,
            jwk,
            eab,
            created_at: Utc::now(),
        };
        acct.validate()?;
        self.account_by_thumbprint
            .insert(dedupe_key, acct.id.clone());
        let id = acct.id.clone();
        self.accounts.insert(id.clone(), acct);
        Ok(id)
    }

    /// Cite: RFC 8555 §7.3.6 — account deactivation is irreversible from
    /// the server side; subsequent JWS requests under that key MUST
    /// return `urn:ietf:params:acme:error:unauthorized`.
    pub fn deactivate_account(&mut self, requesting_tenant: &str, id: &str) -> AcmeResult<()> {
        let acct = self.account_mut(requesting_tenant, id)?;
        acct.status = AccountStatus::Deactivated;
        Ok(())
    }

    pub fn account(&self, requesting_tenant: &str, id: &str) -> AcmeResult<&Account> {
        let acct = self
            .accounts
            .get(id)
            .ok_or_else(|| AcmeError::AccountNotFound(id.to_string()))?;
        if acct.tenant_id != requesting_tenant {
            return Err(AcmeError::CrossTenantDenied {
                store: acct.tenant_id.clone(),
                req: requesting_tenant.to_string(),
            });
        }
        Ok(acct)
    }

    fn account_mut(&mut self, requesting_tenant: &str, id: &str) -> AcmeResult<&mut Account> {
        let acct = self
            .accounts
            .get_mut(id)
            .ok_or_else(|| AcmeError::AccountNotFound(id.to_string()))?;
        if acct.tenant_id != requesting_tenant {
            return Err(AcmeError::CrossTenantDenied {
                store: acct.tenant_id.clone(),
                req: requesting_tenant.to_string(),
            });
        }
        Ok(acct)
    }

    /// Cite: RFC 8555 §7.4 (newOrder). The server creates one
    /// Authorization per identifier with one Challenge per supported type.
    pub fn new_order(
        &mut self,
        requesting_tenant: &str,
        account_id: &str,
        identifiers: Vec<Identifier>,
    ) -> AcmeResult<String> {
        let acct = self.account(requesting_tenant, account_id)?;
        if acct.status != AccountStatus::Valid {
            return Err(AcmeError::Unauthorized(format!(
                "account {} is {:?}",
                account_id, acct.status,
            )));
        }
        let order_id = Uuid::new_v4().to_string();
        let mut order = Order::new(order_id.clone(), requesting_tenant, account_id, identifiers);
        order.validate_identifiers()?;

        let mut authz_ids = Vec::new();
        for ident in &order.identifiers {
            let authz_id = Uuid::new_v4().to_string();
            let challenges = vec![
                make_challenge(&authz_id, ChallengeType::Http01),
                make_challenge(&authz_id, ChallengeType::Dns01),
                make_challenge(&authz_id, ChallengeType::TlsAlpn01),
            ];
            let authz = Authorization {
                id: authz_id.clone(),
                tenant_id: requesting_tenant.to_string(),
                account_id: account_id.to_string(),
                identifier: ident.clone(),
                status: AuthzStatus::Pending,
                challenges,
                expires: Utc::now() + Duration::hours(24),
            };
            self.authorizations.insert(authz_id.clone(), authz);
            authz_ids.push(authz_id);
        }
        order.authorization_ids = authz_ids;
        self.orders.insert(order_id.clone(), order);
        Ok(order_id)
    }

    pub fn order(&self, requesting_tenant: &str, id: &str) -> AcmeResult<&Order> {
        let order = self
            .orders
            .get(id)
            .ok_or_else(|| AcmeError::Malformed(format!("order {} not found", id)))?;
        if order.tenant_id != requesting_tenant {
            return Err(AcmeError::CrossTenantDenied {
                store: order.tenant_id.clone(),
                req: requesting_tenant.to_string(),
            });
        }
        Ok(order)
    }

    /// Cite: RFC 8555 §7.5.1 (Responding to Challenges) — when the server
    /// successfully validates a challenge it transitions the challenge to
    /// `valid`, the parent authorization to `valid`, and (when every
    /// authorization on the order is `valid`) the order to `ready`.
    pub fn mark_challenge_valid(
        &mut self,
        requesting_tenant: &str,
        challenge_id: &str,
    ) -> AcmeResult<()> {
        // Find authz containing this challenge
        let authz_id = self
            .authorizations
            .iter()
            .find(|(_, a)| {
                a.tenant_id == requesting_tenant
                    && a.challenges.iter().any(|c| c.id == challenge_id)
            })
            .map(|(id, _)| id.clone())
            .ok_or_else(|| {
                AcmeError::ChallengeInvalid(
                    challenge_id.to_string(),
                    "challenge or tenant mismatch".into(),
                )
            })?;
        let authz = self.authorizations.get_mut(&authz_id).unwrap();
        for ch in authz.challenges.iter_mut() {
            if ch.id == challenge_id {
                ch.status = ChallengeStatus::Valid;
                ch.validated_at = Some(Utc::now());
            }
        }
        authz.status = AuthzStatus::Valid;

        // Promote the order if every authorization is now valid
        let order_id = self
            .orders
            .values()
            .find(|o| o.authorization_ids.iter().any(|id| id == &authz_id))
            .map(|o| o.id.clone());
        if let Some(order_id) = order_id {
            let all_valid = {
                let order = &self.orders[&order_id];
                order.authorization_ids.iter().all(|aid| {
                    self.authorizations
                        .get(aid)
                        .map(|a| a.status == AuthzStatus::Valid)
                        .unwrap_or(false)
                })
            };
            if all_valid {
                let order = self.orders.get_mut(&order_id).unwrap();
                if order.status.can_transition_to(OrderStatus::Ready) {
                    order.status = OrderStatus::Ready;
                }
            }
        }
        Ok(())
    }

    /// Cite: RFC 8555 §7.4 (finalize) — moves a Ready order through
    /// Processing → Valid and stamps the certificate URL. The CSR is
    /// validated by the caller (see cave-pki).
    pub fn finalize_order(
        &mut self,
        requesting_tenant: &str,
        order_id: &str,
        certificate_url: impl Into<String>,
    ) -> AcmeResult<()> {
        let order = self
            .orders
            .get_mut(order_id)
            .ok_or_else(|| AcmeError::Malformed(format!("order {} not found", order_id)))?;
        if order.tenant_id != requesting_tenant {
            return Err(AcmeError::CrossTenantDenied {
                store: order.tenant_id.clone(),
                req: requesting_tenant.to_string(),
            });
        }
        if order.status != OrderStatus::Ready {
            return Err(AcmeError::OrderNotReady(
                order_id.to_string(),
                format!("{:?}", order.status),
            ));
        }
        order.status = OrderStatus::Processing;
        order.certificate_url = Some(certificate_url.into());
        order.status = OrderStatus::Valid;
        Ok(())
    }

    pub fn account_count(&self) -> usize {
        self.accounts.len()
    }
    pub fn order_count(&self) -> usize {
        self.orders.len()
    }
    pub fn authorization_count(&self) -> usize {
        self.authorizations.len()
    }
}

fn make_challenge(authz_id: &str, kind: ChallengeType) -> Challenge {
    use base64::Engine as _;
    let token = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Uuid::new_v4().as_bytes());
    Challenge {
        id: Uuid::new_v4().to_string(),
        kind,
        status: ChallengeStatus::Pending,
        url: format!("/acme/chall/{}/{}", authz_id, kind.as_str()),
        token,
        validated_at: None,
        error: None,
    }
}

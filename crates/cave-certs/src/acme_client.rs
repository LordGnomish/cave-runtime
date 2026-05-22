// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Local ACME client — drives an in-process [`cave_acme::AcmeServer`]
//! through the new-account / new-order / challenge / finalize / cert
//! workflow.
//!
//! The HTTP transport (POST-as-GET, JWS, replay nonce) lives in the
//! cave-gateway / cave-runtime layer; this module is the stateful
//! orchestrator that drives the protocol.
//!
//! Cite: RFC 8555 §7 (Account / Order / Authorization), cert-manager
//! `pkg/issuer/acme/order/order.go::syncOrderStatus`.

use cave_acme::{
    Account, AcmeError, AcmeResult, AcmeServer, ChallengeStatus, ChallengeType, Identifier, Jwk,
    OrderStatus,
};

#[derive(Debug)]
pub struct AcmeClient<'a> {
    pub server: &'a mut AcmeServer,
    pub tenant_id: String,
    pub account_id: String,
    pub jwk: Jwk,
}

impl<'a> AcmeClient<'a> {
    /// Cite: RFC 8555 §7.3 — register-or-reuse account using the JWK
    /// thumbprint as the dedupe key.
    pub fn register(
        server: &'a mut AcmeServer,
        tenant_id: impl Into<String>,
        jwk: Jwk,
        contact: Vec<String>,
    ) -> AcmeResult<Self> {
        let tenant_id = tenant_id.into();
        let account_id = server.new_account(tenant_id.clone(), jwk.clone(), contact, true, None)?;
        Ok(Self {
            server,
            tenant_id,
            account_id,
            jwk,
        })
    }

    pub fn account(&self) -> AcmeResult<&Account> {
        self.server.account(&self.tenant_id, &self.account_id)
    }

    /// Cite: RFC 8555 §7.4 (newOrder) — submit DNS identifiers; server
    /// returns the order id which is the URL slug for finalize/cert.
    pub fn new_order(&mut self, dns_names: &[&str]) -> AcmeResult<String> {
        let identifiers: Vec<Identifier> = dns_names.iter().map(|n| Identifier::dns(*n)).collect();
        self.server
            .new_order(&self.tenant_id, &self.account_id, identifiers)
    }

    /// Cite: RFC 8555 §8 — drive every authorization on the order to
    /// `valid` by satisfying the named challenge type. Returns the list
    /// of challenge ids that were validated.
    pub fn solve_challenges(
        &mut self,
        order_id: &str,
        kind: ChallengeType,
    ) -> AcmeResult<Vec<String>> {
        let authz_ids: Vec<String> = self
            .server
            .order(&self.tenant_id, order_id)?
            .authorization_ids
            .clone();
        let mut solved = Vec::new();
        for aid in &authz_ids {
            // We don't expose authorization() publicly on AcmeServer,
            // so rely on mark_challenge_valid finding the right one.
            let order = self.server.order(&self.tenant_id, order_id)?;
            let _ = order;
            // Find the challenge of the requested kind by walking the
            // authz lookup via the public mark_challenge_valid path.
            let challenge_id = self.find_challenge_id(aid, kind)?;
            self.server
                .mark_challenge_valid(&self.tenant_id, &challenge_id)?;
            solved.push(challenge_id);
        }
        Ok(solved)
    }

    fn find_challenge_id(&self, _authz_id: &str, _kind: ChallengeType) -> AcmeResult<String> {
        // Authorizations and their challenges are private to AcmeServer
        // today. The helper `solve_challenges` would normally reach into
        // a public iterator; for the scaffold it is wired in the
        // integration-test path via a manual probe (see tests). Mark
        // this branch with an error so callers know the surface is
        // pending.
        Err(AcmeError::Malformed(
            "AcmeClient::solve_challenges requires AcmeServer to expose authorization iterators \
             — landing in a follow-up batch"
                .into(),
        ))
    }

    /// Cite: RFC 8555 §7.4 (finalize) — once every authz is valid the
    /// order moves to Ready; finalize then submits the CSR and
    /// transitions the order to Valid with the cert URL populated.
    pub fn finalize(
        &mut self,
        order_id: &str,
        certificate_url: impl Into<String>,
    ) -> AcmeResult<()> {
        self.server
            .finalize_order(&self.tenant_id, order_id, certificate_url)
    }

    /// Cite: RFC 8555 §7.4 — order status is consulted by the renewal
    /// controller to decide whether to wait, retry or give up.
    pub fn order_status(&self, order_id: &str) -> AcmeResult<OrderStatus> {
        Ok(self.server.order(&self.tenant_id, order_id)?.status)
    }

    pub fn challenge_status_count(
        &self,
        order_id: &str,
        status: ChallengeStatus,
    ) -> AcmeResult<usize> {
        let _ = (order_id, status); // tracked via order status; placeholder.
        Ok(0)
    }
}

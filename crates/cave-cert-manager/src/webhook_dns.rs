// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! ACME webhook DNS-01 solver protocol — in-process port of
//! cert-manager's out-of-tree DNS solver contract.
//!
//! Cite: `pkg/acme/webhook/apis/acme/v1alpha1/types.go` +
//! `pkg/acme/webhook/webhook.go`. Upstream serves the
//! `ChallengeRequest`/`ChallengeResponse` pair over a Kubernetes
//! apiserver-style webhook; cave-cert-manager carries the same
//! protocol types and a `WebhookSolverRegistry` that dispatches
//! `Present`/`CleanUp` to registered solvers in-process — the gRPC /
//! apiserver transport itself stays in cave-net.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Cite: `ChallengeAction` constants in
/// `pkg/acme/webhook/apis/acme/v1alpha1/types.go`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChallengeAction {
    /// `ChallengeActionPresent` — publish the DNS-01 TXT record.
    Present,
    /// `ChallengeActionCleanUp` — remove a previously-presented record.
    CleanUp,
}

impl ChallengeAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Present => "Present",
            Self::CleanUp => "CleanUp",
        }
    }
}

/// Cite: `ChallengeRequest` in
/// `pkg/acme/webhook/apis/acme/v1alpha1/types.go`. Field names mirror
/// the upstream JSON tags so an out-of-tree solver config lifts
/// mechanically.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChallengeRequest {
    /// `uid` — opaque per-request id echoed back in the response.
    pub uid: String,
    /// `action` — Present or CleanUp.
    pub action: ChallengeAction,
    /// `type` — always `dns-01` for the webhook DNS solver.
    #[serde(rename = "type")]
    pub kind: String,
    /// `dnsName` — the identifier being validated (e.g. `example.com`).
    pub dns_name: String,
    /// `key` — the base64url SHA-256 digest published as the TXT value.
    pub key: String,
    /// `resourceNamespace` — the namespace the Issuer/Secret lives in.
    pub resource_namespace: String,
    /// `resolvedFQDN` — fully-qualified `_acme-challenge.<dnsName>.`.
    pub resolved_fqdn: String,
    /// `resolvedZone` — the authoritative zone the FQDN sits under.
    pub resolved_zone: String,
    /// `allowAmbientCredentials` — may the solver read ambient cloud creds.
    pub allow_ambient_credentials: bool,
    /// `config` — opaque per-solver configuration (apiextensions JSON).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<serde_json::Value>,
}

impl ChallengeRequest {
    /// Build a DNS-01 `Present` request for `dns_name`, mirroring
    /// cert-manager's `_acme-challenge.<domain>.` FQDN convention
    /// (RFC 8555 §8.4). `key` is the already-computed TXT digest.
    pub fn for_dns01(uid: &str, dns_name: &str, key: &str, namespace: &str) -> Self {
        let trimmed = dns_name.trim_end_matches('.');
        Self {
            uid: uid.to_string(),
            action: ChallengeAction::Present,
            kind: "dns-01".to_string(),
            dns_name: dns_name.to_string(),
            key: key.to_string(),
            resource_namespace: namespace.to_string(),
            resolved_fqdn: format!("_acme-challenge.{}.", trimmed),
            resolved_zone: String::new(),
            allow_ambient_credentials: false,
            config: None,
        }
    }
}

/// Cite: `metav1.Status` carried on `ChallengeResponse.status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResponseStatus {
    /// `reason` — machine-readable cause (`NotFound`, `Failure`, …).
    pub reason: String,
    /// `message` — human-readable detail.
    pub message: String,
}

/// Cite: `ChallengeResponse` in
/// `pkg/acme/webhook/apis/acme/v1alpha1/types.go`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChallengeResponse {
    /// `uid` — echoes the request UID.
    pub uid: String,
    /// `success` — true once the solver published / removed the record.
    pub success: bool,
    /// `status` — populated only on failure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<ResponseStatus>,
}

/// Cite: the `Solver` interface in `pkg/acme/webhook/webhook.go`.
/// Out-of-tree DNS providers implement this; the registry drives them.
pub trait WebhookDnsSolver {
    /// Group name the solver answers for (e.g. `acme.example.com`).
    fn name(&self) -> &str;
    /// Publish the DNS-01 TXT record for the challenge.
    fn present(&mut self, ch: &ChallengeRequest) -> Result<(), String>;
    /// Remove a previously-presented record.
    fn clean_up(&mut self, ch: &ChallengeRequest) -> Result<(), String>;
}

/// In-process registry — mirrors the upstream webhook apiserver's
/// routing of a `ChallengeRequest` to the solver registered for its
/// API group, then dispatching by `action`.
#[derive(Default)]
pub struct WebhookSolverRegistry {
    solvers: BTreeMap<String, Box<dyn WebhookDnsSolver + Send + Sync>>,
}

impl std::fmt::Debug for WebhookSolverRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebhookSolverRegistry")
            .field("groups", &self.solver_names())
            .finish()
    }
}

impl WebhookSolverRegistry {
    pub fn new() -> Self {
        Self {
            solvers: BTreeMap::new(),
        }
    }

    /// Register (or replace) the solver for `group_name`.
    pub fn register(&mut self, group_name: &str, solver: Box<dyn WebhookDnsSolver + Send + Sync>) {
        self.solvers.insert(group_name.to_string(), solver);
    }

    /// Sorted list of registered group names (BTreeMap keeps order).
    pub fn solver_names(&self) -> Vec<String> {
        self.solvers.keys().cloned().collect()
    }

    /// Route a challenge to the matching solver and dispatch by action.
    /// An unknown group yields a `NotFound` failure; a solver error is
    /// surfaced as a `Failure` status. The request UID is always echoed.
    pub fn dispatch(&mut self, group_name: &str, ch: &ChallengeRequest) -> ChallengeResponse {
        let Some(solver) = self.solvers.get_mut(group_name) else {
            return ChallengeResponse {
                uid: ch.uid.clone(),
                success: false,
                status: Some(ResponseStatus {
                    reason: "NotFound".to_string(),
                    message: format!("no webhook DNS solver registered for group `{group_name}`"),
                }),
            };
        };
        let result = match ch.action {
            ChallengeAction::Present => solver.present(ch),
            ChallengeAction::CleanUp => solver.clean_up(ch),
        };
        match result {
            Ok(()) => ChallengeResponse {
                uid: ch.uid.clone(),
                success: true,
                status: None,
            },
            Err(msg) => ChallengeResponse {
                uid: ch.uid.clone(),
                success: false,
                status: Some(ResponseStatus {
                    reason: "Failure".to_string(),
                    message: msg,
                }),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// Recording fake — proves the registry dispatches the right action
    /// to the right solver and echoes the request UID. `Send + Sync`
    /// (Arc/Mutex) so it satisfies the registry's solver bound.
    #[derive(Default)]
    struct RecordingSolver {
        presented: Arc<Mutex<Vec<String>>>,
        cleaned: Arc<Mutex<Vec<String>>>,
        fail_present: bool,
    }

    impl WebhookDnsSolver for RecordingSolver {
        fn name(&self) -> &str {
            "recording"
        }
        fn present(&mut self, ch: &ChallengeRequest) -> Result<(), String> {
            if self.fail_present {
                return Err("upstream DNS API rejected the record".into());
            }
            self.presented.lock().unwrap().push(ch.resolved_fqdn.clone());
            Ok(())
        }
        fn clean_up(&mut self, ch: &ChallengeRequest) -> Result<(), String> {
            self.cleaned.lock().unwrap().push(ch.resolved_fqdn.clone());
            Ok(())
        }
    }

    fn req(action: ChallengeAction) -> ChallengeRequest {
        ChallengeRequest {
            uid: "uid-123".into(),
            action,
            kind: "dns-01".into(),
            dns_name: "example.com".into(),
            key: "TXTVALUE".into(),
            resource_namespace: "default".into(),
            resolved_fqdn: "_acme-challenge.example.com.".into(),
            resolved_zone: "example.com.".into(),
            allow_ambient_credentials: false,
            config: None,
        }
    }

    #[test]
    fn challenge_action_round_trips_upstream_strings() {
        assert_eq!(ChallengeAction::Present.as_str(), "Present");
        assert_eq!(ChallengeAction::CleanUp.as_str(), "CleanUp");
    }

    #[test]
    fn challenge_request_for_dns01_builds_underscore_acme_fqdn() {
        let ch = ChallengeRequest::for_dns01("uid-9", "www.example.com", "DIGEST", "ns-a");
        assert_eq!(ch.resolved_fqdn, "_acme-challenge.www.example.com.");
        assert_eq!(ch.kind, "dns-01");
        assert_eq!(ch.key, "DIGEST");
        assert_eq!(ch.resource_namespace, "ns-a");
        assert!(matches!(ch.action, ChallengeAction::Present));
    }

    #[test]
    fn dispatch_present_calls_solver_and_echoes_uid() {
        let presented = Arc::new(Mutex::new(Vec::new()));
        let solver = RecordingSolver {
            presented: presented.clone(),
            ..Default::default()
        };
        let mut reg = WebhookSolverRegistry::new();
        reg.register("acme.example.com", Box::new(solver));
        let resp = reg.dispatch("acme.example.com", &req(ChallengeAction::Present));
        assert!(resp.success);
        assert_eq!(resp.uid, "uid-123");
        assert_eq!(
            presented.lock().unwrap().as_slice(),
            &["_acme-challenge.example.com."]
        );
    }

    #[test]
    fn dispatch_cleanup_routes_to_clean_up() {
        let cleaned = Arc::new(Mutex::new(Vec::new()));
        let solver = RecordingSolver {
            cleaned: cleaned.clone(),
            ..Default::default()
        };
        let mut reg = WebhookSolverRegistry::new();
        reg.register("acme.example.com", Box::new(solver));
        let resp = reg.dispatch("acme.example.com", &req(ChallengeAction::CleanUp));
        assert!(resp.success);
        assert_eq!(cleaned.lock().unwrap().len(), 1);
    }

    #[test]
    fn dispatch_unknown_group_fails_with_notfound_status() {
        let mut reg = WebhookSolverRegistry::new();
        let resp = reg.dispatch("nope.example.com", &req(ChallengeAction::Present));
        assert!(!resp.success);
        let st = resp.status.expect("a failure must carry a Status");
        assert_eq!(st.reason, "NotFound");
        assert!(st.message.contains("nope.example.com"));
    }

    #[test]
    fn dispatch_present_failure_surfaces_solver_error_message() {
        let solver = RecordingSolver {
            fail_present: true,
            ..Default::default()
        };
        let mut reg = WebhookSolverRegistry::new();
        reg.register("acme.example.com", Box::new(solver));
        let resp = reg.dispatch("acme.example.com", &req(ChallengeAction::Present));
        assert!(!resp.success);
        let st = resp.status.expect("failure status");
        assert_eq!(st.reason, "Failure");
        assert!(st.message.contains("upstream DNS API rejected"));
    }

    #[test]
    fn solver_names_lists_registered_groups_sorted() {
        let mut reg = WebhookSolverRegistry::new();
        reg.register("z.example.com", Box::new(RecordingSolver::default()));
        reg.register("a.example.com", Box::new(RecordingSolver::default()));
        assert_eq!(
            reg.solver_names(),
            vec!["a.example.com".to_string(), "z.example.com".to_string()]
        );
    }

    #[test]
    fn registering_same_group_twice_replaces_solver() {
        let mut reg = WebhookSolverRegistry::new();
        reg.register("dup.example.com", Box::new(RecordingSolver::default()));
        reg.register("dup.example.com", Box::new(RecordingSolver::default()));
        assert_eq!(reg.solver_names().len(), 1);
    }
}

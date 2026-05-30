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

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// Recording fake — proves the registry dispatches the right action
    /// to the right solver and echoes the request UID.
    #[derive(Default)]
    struct RecordingSolver {
        presented: Rc<RefCell<Vec<String>>>,
        cleaned: Rc<RefCell<Vec<String>>>,
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
            self.presented.borrow_mut().push(ch.resolved_fqdn.clone());
            Ok(())
        }
        fn clean_up(&mut self, ch: &ChallengeRequest) -> Result<(), String> {
            self.cleaned.borrow_mut().push(ch.resolved_fqdn.clone());
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
        let presented = Rc::new(RefCell::new(Vec::new()));
        let solver = RecordingSolver {
            presented: presented.clone(),
            ..Default::default()
        };
        let mut reg = WebhookSolverRegistry::new();
        reg.register("acme.example.com", Box::new(solver));
        let resp = reg.dispatch("acme.example.com", &req(ChallengeAction::Present));
        assert!(resp.success);
        assert_eq!(resp.uid, "uid-123");
        assert_eq!(presented.borrow().as_slice(), &["_acme-challenge.example.com."]);
    }

    #[test]
    fn dispatch_cleanup_routes_to_clean_up() {
        let cleaned = Rc::new(RefCell::new(Vec::new()));
        let solver = RecordingSolver {
            cleaned: cleaned.clone(),
            ..Default::default()
        };
        let mut reg = WebhookSolverRegistry::new();
        reg.register("acme.example.com", Box::new(solver));
        let resp = reg.dispatch("acme.example.com", &req(ChallengeAction::CleanUp));
        assert!(resp.success);
        assert_eq!(cleaned.borrow().len(), 1);
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

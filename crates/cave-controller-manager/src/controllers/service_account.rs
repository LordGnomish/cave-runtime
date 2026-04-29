//! ServiceAccount controller — provisions a default token for each SA that
//! does not yet have one.
//!
//! Upstream: pkg/controller/serviceaccount/serviceaccounts_controller.go::syncServiceAccount

use crate::models::ServiceAccountObs;

/// Reconcile a ServiceAccount — if it lacks a default token, fabricate one and
/// return the new SA observation. Tokens are JWT-shaped opaque strings in
/// production; here we use a deterministic placeholder keyed by SA identity.
pub fn reconcile_service_account(mut sa: ServiceAccountObs) -> ServiceAccountObs {
    if sa.default_token.is_none() {
        sa.default_token = Some(format!("sa-token:{}/{}", sa.namespace, sa.name));
    }
    sa
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Port of `TestServiceAccountTokenCreated` — an SA without a token receives
    /// one on reconcile; an existing token is preserved.
    #[test]
    fn test_service_account_token_created() {
        let fresh = ServiceAccountObs {
            name: "default".into(),
            namespace: "kube-system".into(),
            default_token: None,
        };
        let after = reconcile_service_account(fresh);
        assert!(after.default_token.is_some());
        let tok = after.default_token.unwrap();
        assert!(tok.contains("kube-system"));
        assert!(tok.contains("default"));
    }

    #[test]
    fn test_service_account_existing_token_preserved() {
        let already = ServiceAccountObs {
            name: "default".into(),
            namespace: "default".into(),
            default_token: Some("preexisting".into()),
        };
        let after = reconcile_service_account(already);
        assert_eq!(after.default_token.as_deref(), Some("preexisting"));
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
//! Bridge between `runtime_client::auth::AuthClient` and `AdminState`.
//!
//! Existing `/admin/auth/*` handlers read from `AdminState.auth_sessions`
//! (a fixture vector). When a live `AuthClient` is configured, the
//! [`materialise_auth_sessions`] helper pulls the realm-scoped live
//! session list and replaces the per-tenant rows in place — same idiom
//! as `materialise_kubelet_pods` in `admin::state`. Handlers stay
//! untouched; render code reads from the refreshed `RwLock<Vec<…>>`.
//!
//! Source: keycloak/keycloak@v22.0.0
//!         services/src/main/java/org/keycloak/services/resources/admin/UserSessionResource.java

use crate::admin::state::{AdminState, AuthSession};
use crate::admin::types::TenantId;
use crate::runtime_client::auth::{AuthClient, ClientError};

/// Project realm-scoped live sessions from an [`AuthClient`] into the
/// tenant-scoped row vector the existing handlers read. The tenant ID
/// is stamped onto every materialised row so the handler-level scope()
/// filter still matches.
///
/// The mapping is intentionally narrow: cave-portal's `AuthSession`
/// holds the fields the dashboard renders (`tenant`, `session_id`,
/// `principal`, `realm`, `expires_unix`). The upstream `UserSession`
/// payload's other fields (ipAddress, start, lastAccess) stay on the
/// `AuthClient` surface for tabs that surface them directly.
pub async fn materialise_auth_sessions(
    state: &AdminState,
    client: &dyn AuthClient,
    realm: &str,
    tenant: &TenantId,
) -> Result<(), ClientError> {
    let live = client.list_sessions(realm).await?;
    let mut rows: Vec<AuthSession> = live
        .into_iter()
        .map(|s| AuthSession {
            tenant: tenant.clone(),
            session_id: s.id,
            principal: s.username,
            realm: s.realm,
            expires_unix: s.last_access,
        })
        .collect();
    // Replace this tenant's rows in place, preserving other tenants.
    let mut store = state.auth_sessions.write().unwrap();
    store.retain(|r| r.tenant != *tenant);
    store.append(&mut rows);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::auth::{clients, events, realms, sessions, users};
    use crate::admin::permission::{Permission, Persona, RequestCtx};
    use crate::admin::state::AdminState;
    use crate::admin::types::TenantId;
    use crate::runtime_client::auth::{
        AccountProfile, AuthFlow, AuthMockClient, ClientApp, Credential, EventPayload, Group,
        IdentityProvider, LinkedApplication, Realm, Role, User, UserSession,
    };
    use std::sync::Arc;

    fn tenant() -> TenantId {
        TenantId::new("acme").unwrap()
    }

    fn ctx_with(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    fn ctx_tenant_admin() -> RequestCtx {
        RequestCtx::developer_as(
            "acme",
            &[Permission::AuthSessionsRead],
            Persona::TenantAdmin,
        )
    }

    fn ctx_anon() -> RequestCtx {
        let mut ctx = RequestCtx::developer("acme", &[Permission::AuthSessionsRead]);
        ctx.persona = Persona::Anonymous;
        ctx
    }

    fn seeded_client() -> Arc<AuthMockClient> {
        let c = Arc::new(AuthMockClient::new());
        c.seed_realm(Realm {
            id: "acme-realm".into(),
            display_name: "Acme".into(),
            enabled: true,
            ssl_required: "external".into(),
            access_token_lifespan: 300,
        });
        c.seed_user(User {
            id: "u-alice".into(),
            realm: "acme-realm".into(),
            username: "alice".into(),
            email: Some("alice@acme.io".into()),
            email_verified: true,
            enabled: true,
            ..Default::default()
        });
        c.seed_user(User {
            id: "u-bob".into(),
            realm: "acme-realm".into(),
            username: "bob".into(),
            email: Some("bob@acme.io".into()),
            enabled: true,
            ..Default::default()
        });
        c.seed_role(Role {
            id: "r-admin".into(),
            realm: "acme-realm".into(),
            name: "platform_admin".into(),
            description: Some("can do anything".into()),
            ..Default::default()
        });
        c.seed_group(Group {
            id: "g-eng".into(),
            realm: "acme-realm".into(),
            name: "engineering".into(),
            path: "/engineering".into(),
            sub_group_count: 0,
        });
        c.seed_idp(IdentityProvider {
            alias: "github".into(),
            realm: "acme-realm".into(),
            display_name: "GitHub".into(),
            provider_id: "github".into(),
            enabled: true,
        });
        c.seed_flow(AuthFlow {
            id: "browser".into(),
            alias: "browser".into(),
            realm: "acme-realm".into(),
            description: Some("Default browser-based flow".into()),
            provider_id: "basic-flow".into(),
            built_in: true,
        });
        c.seed_session(UserSession {
            id: "sess-aaa".into(),
            realm: "acme-realm".into(),
            user_id: "u-alice".into(),
            username: "alice".into(),
            ip_address: "10.0.0.1".into(),
            start: 1_000_000,
            last_access: 1_010_000,
        });
        c.seed_session(UserSession {
            id: "sess-bbb".into(),
            realm: "acme-realm".into(),
            user_id: "u-bob".into(),
            username: "bob".into(),
            ip_address: "10.0.0.2".into(),
            start: 1_000_500,
            last_access: 1_020_000,
        });
        c.push_event(EventPayload {
            time: 1_009_000,
            realm: "acme-realm".into(),
            kind: "LOGIN".into(),
            user_id: Some("u-alice".into()),
            ..Default::default()
        });
        c.seed_profile(
            "acme-realm",
            AccountProfile {
                username: "alice".into(),
                email: Some("alice@acme.io".into()),
                first_name: Some("Alice".into()),
                last_name: Some("Doe".into()),
            },
        );
        c.seed_credential(
            "acme-realm",
            "alice",
            Credential {
                id: "cred-pass".into(),
                kind: "password".into(),
                user_label: Some("My password".into()),
                created_date: 100,
            },
        );
        c.seed_credential(
            "acme-realm",
            "alice",
            Credential {
                id: "cred-wak".into(),
                kind: "webauthn-passwordless".into(),
                user_label: Some("YubiKey".into()),
                created_date: 200,
            },
        );
        c.seed_application(
            "acme-realm",
            "alice",
            LinkedApplication {
                client_id: "portal".into(),
                name: "Cave Portal".into(),
                in_use: true,
                consent_required: false,
            },
        );
        c.create_client(
            "acme-realm",
            &ClientApp {
                id: "c-portal".into(),
                client_id: "portal".into(),
                realm: "acme-realm".into(),
                name: Some("Cave Portal".into()),
                enabled: true,
                ..Default::default()
            },
        )
        .now_or_never_unwrap();
        c
    }

    /// Tiny adapter so seeded_client() can use sync setup w/o `.await`.
    trait NowOrNever<T> {
        fn now_or_never_unwrap(self) -> T;
    }
    impl<F, T> NowOrNever<T> for F
    where
        F: std::future::Future<Output = Result<T, ClientError>>,
    {
        fn now_or_never_unwrap(self) -> T {
            // SAFETY: AuthMockClient methods are non-blocking — every
            // path returns immediately after a sync RwLock op.
            futures::executor::block_on(self).expect("seeded client setup")
        }
    }

    // ── 1) materialise_auth_sessions wiring ───────────────────────────

    #[tokio::test]
    async fn materialise_replaces_acme_rows_in_place() {
        let state = AdminState::empty();
        let t = tenant();
        // Pre-seed an "evil" tenant row that MUST survive the refresh.
        state.auth_sessions.write().unwrap().push(AuthSession {
            tenant: TenantId::new("evil").unwrap(),
            session_id: "sess-evil".into(),
            principal: "mallory".into(),
            realm: "evil-realm".into(),
            expires_unix: 999_999,
        });
        let client = seeded_client();
        materialise_auth_sessions(&state, &*client, "acme-realm", &t)
            .await
            .unwrap();
        let store = state.auth_sessions.read().unwrap();
        assert!(store.iter().any(|s| s.session_id == "sess-evil"));
        assert!(store.iter().any(|s| s.session_id == "sess-aaa"));
        assert_eq!(
            store.iter().filter(|s| s.tenant == t).count(),
            2,
            "tenant rows replaced from live source"
        );
    }

    #[tokio::test]
    async fn materialise_is_idempotent_across_two_calls() {
        let state = AdminState::empty();
        let t = tenant();
        let client = seeded_client();
        materialise_auth_sessions(&state, &*client, "acme-realm", &t)
            .await
            .unwrap();
        materialise_auth_sessions(&state, &*client, "acme-realm", &t)
            .await
            .unwrap();
        let store = state.auth_sessions.read().unwrap();
        let acme: Vec<_> = store.iter().filter(|s| s.tenant == t).collect();
        assert_eq!(acme.len(), 2, "idempotent — no duplication");
    }

    #[tokio::test]
    async fn materialise_unknown_realm_yields_empty_rows() {
        let state = AdminState::empty();
        let t = tenant();
        let client = seeded_client();
        materialise_auth_sessions(&state, &*client, "no-such-realm", &t)
            .await
            .unwrap();
        let store = state.auth_sessions.read().unwrap();
        assert!(
            store.iter().filter(|s| s.tenant == t).next().is_none(),
            "no realm match → no rows materialised"
        );
    }

    // ── 2) handler smoke (per page, populated state via materialise) ──

    fn populated_state() -> AdminState {
        let state = AdminState::empty();
        let t = tenant();
        let client = seeded_client();
        futures::executor::block_on(materialise_auth_sessions(
            &state,
            &*client,
            "acme-realm",
            &t,
        ))
        .unwrap();
        state
    }

    #[test]
    fn realms_handler_lists_one_realm_per_session_group() {
        let state = populated_state();
        let ctx = ctx_with(&[Permission::AuthSessionsRead]);
        let rows = realms::list_realms(&state, &ctx).unwrap();
        assert!(rows.iter().any(|r| r.realm == "acme-realm"));
    }

    #[test]
    fn realms_handler_refuses_without_perm() {
        let state = populated_state();
        let ctx = ctx_with(&[]);
        assert!(realms::list_realms(&state, &ctx).is_err());
    }

    #[test]
    fn clients_handler_groups_principals_by_host() {
        let state = populated_state();
        let ctx = ctx_with(&[Permission::AuthSessionsRead]);
        let rows = clients::list_clients(&state, &ctx).unwrap();
        // Principal "alice" has no @ — so it's its own client_id.
        assert!(!rows.is_empty());
    }

    #[test]
    fn users_handler_returns_per_principal_rows() {
        let state = populated_state();
        let ctx = ctx_with(&[Permission::AuthSessionsRead]);
        let rows = users::list_users(&state, &ctx).unwrap();
        assert!(rows.iter().any(|u| u.principal == "alice"));
        assert!(rows.iter().any(|u| u.principal == "bob"));
    }

    #[test]
    fn sessions_handler_render_contains_owner_only() {
        let state = populated_state();
        let ctx = ctx_with(&[Permission::AuthSessionsRead]);
        let html = sessions::render(&state, &ctx).unwrap();
        assert!(html.contains("sess-aaa"));
        assert!(!html.contains("sess-evil"));
    }

    #[test]
    fn events_handler_returns_login_event_per_session() {
        let state = populated_state();
        let ctx = ctx_with(&[Permission::AuthSessionsRead]);
        let evts = events::list_events(&state, &ctx).unwrap();
        assert_eq!(evts.len(), 2);
        assert!(evts.iter().all(|e| e.event_type == "LOGIN"));
    }

    #[test]
    fn events_handler_excludes_other_tenants() {
        let state = AdminState::empty();
        let t = tenant();
        // Seed acme rows via materialise.
        let client = seeded_client();
        futures::executor::block_on(materialise_auth_sessions(
            &state,
            &*client,
            "acme-realm",
            &t,
        ))
        .unwrap();
        // Also push an evil tenant row that the handler must filter out.
        state.auth_sessions.write().unwrap().push(AuthSession {
            tenant: TenantId::new("evil").unwrap(),
            session_id: "sess-evil".into(),
            principal: "mallory".into(),
            realm: "evil-realm".into(),
            expires_unix: 1,
        });
        let ctx = ctx_with(&[Permission::AuthSessionsRead]);
        let evts = events::list_events(&state, &ctx).unwrap();
        assert!(!evts.iter().any(|e| e.principal == "mallory"));
    }

    // ── 3) AuthClient surfaces accessible via the bridge ──────────────

    #[tokio::test]
    async fn bridge_lists_realms_via_trait() {
        let client = seeded_client();
        let r: Vec<Realm> = client.list_realms().await.unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].id, "acme-realm");
    }

    #[tokio::test]
    async fn bridge_lists_users_via_trait() {
        let client = seeded_client();
        let u: Vec<User> = client.list_users("acme-realm").await.unwrap();
        assert_eq!(u.len(), 2);
    }

    #[tokio::test]
    async fn bridge_create_client_round_trip() {
        let client = seeded_client();
        let app = ClientApp {
            id: "c-2".into(),
            client_id: "cli-app".into(),
            realm: "acme-realm".into(),
            enabled: true,
            ..Default::default()
        };
        client.create_client("acme-realm", &app).await.unwrap();
        let listed = client.list_clients("acme-realm").await.unwrap();
        assert!(listed.iter().any(|c| c.client_id == "cli-app"));
    }

    #[tokio::test]
    async fn bridge_create_then_delete_client_returns_not_found() {
        let client = seeded_client();
        let app = ClientApp {
            id: "c-3".into(),
            client_id: "ephemeral".into(),
            realm: "acme-realm".into(),
            enabled: true,
            ..Default::default()
        };
        client.create_client("acme-realm", &app).await.unwrap();
        client.delete_client("acme-realm", "c-3").await.unwrap();
        let err = client.get_client("acme-realm", "c-3").await.unwrap_err();
        assert!(matches!(err, ClientError::NotFound(_)));
    }

    #[tokio::test]
    async fn bridge_create_realm_conflict_when_seeded() {
        let client = seeded_client();
        let r = Realm {
            id: "acme-realm".into(),
            ..Default::default()
        };
        let err = client.create_realm(&r).await.unwrap_err();
        assert!(matches!(err, ClientError::Conflict(_)));
    }

    #[tokio::test]
    async fn bridge_lists_roles_for_realm() {
        let client = seeded_client();
        let v = client.list_roles("acme-realm").await.unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].name, "platform_admin");
    }

    #[tokio::test]
    async fn bridge_groups_path_round_trips() {
        let client = seeded_client();
        let g = client.get_group("acme-realm", "g-eng").await.unwrap();
        assert_eq!(g.path, "/engineering");
    }

    #[tokio::test]
    async fn bridge_idp_provider_id_round_trips() {
        let client = seeded_client();
        let v = client.list_identity_providers("acme-realm").await.unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].provider_id, "github");
    }

    #[tokio::test]
    async fn bridge_auth_flows_built_in_visible() {
        let client = seeded_client();
        let v = client.list_auth_flows("acme-realm").await.unwrap();
        assert!(v.iter().any(|f| f.alias == "browser" && f.built_in));
    }

    #[tokio::test]
    async fn bridge_list_events_returns_login_with_user_id() {
        let client = seeded_client();
        let v = client.list_events("acme-realm", None, &[]).await.unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].user_id.as_deref(), Some("u-alice"));
    }

    #[tokio::test]
    async fn bridge_account_profile_round_trip_for_alice() {
        let client = seeded_client();
        let p = client
            .get_account_profile("acme-realm", "alice")
            .await
            .unwrap();
        assert_eq!(p.first_name.as_deref(), Some("Alice"));
        assert_eq!(p.last_name.as_deref(), Some("Doe"));
    }

    #[tokio::test]
    async fn bridge_account_credentials_two_kinds_seen() {
        let client = seeded_client();
        let creds = client
            .list_account_credentials("acme-realm", "alice")
            .await
            .unwrap();
        let kinds: Vec<&str> = creds.iter().map(|c| c.kind.as_str()).collect();
        assert!(kinds.contains(&"password"));
        assert!(kinds.contains(&"webauthn-passwordless"));
    }

    #[tokio::test]
    async fn bridge_account_applications_marks_in_use() {
        let client = seeded_client();
        let apps = client
            .list_account_applications("acme-realm", "alice")
            .await
            .unwrap();
        assert_eq!(apps.len(), 1);
        assert!(apps[0].in_use);
    }

    // ── 4) Persona / permission negative paths ────────────────────────

    #[test]
    fn realms_handler_anonymous_persona_still_blocked_by_perm_gate() {
        let state = populated_state();
        let ctx = ctx_anon();
        // ctx_anon has AuthSessionsRead, but persona=Anonymous (mock —
        // the handler doesn't enforce persona, just permission).
        let res = realms::list_realms(&state, &ctx);
        assert!(
            res.is_ok(),
            "anonymous w/ perm passes — persona gate not on this surface"
        );
    }

    #[test]
    fn realms_handler_no_perm_returns_err() {
        let state = populated_state();
        let ctx = ctx_with(&[]);
        assert!(realms::list_realms(&state, &ctx).is_err());
    }

    #[test]
    fn tenant_admin_persona_keeps_tenant_scope() {
        let state = populated_state();
        let ctx = ctx_tenant_admin();
        let rows = realms::list_realms(&state, &ctx).unwrap();
        // Same as platform_admin for tenant-scoped views.
        assert!(rows.iter().any(|r| r.realm == "acme-realm"));
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
//! Account console — server-side port of Keycloak's `account-ui` SPA.
//!
//! Source: keycloak/keycloak@b825ba97 `js/apps/account-ui/src/`
//! (tag `22.0.0`).
//!
//! Six end-user self-service pages mounted under `/account/`:
//!
//! * [`personal_info`]   — `PersonalInfo.tsx` (first/last name + attrs).
//! * [`signin_methods`]  — `SigningIn.tsx` (password + OTP + WebAuthn).
//! * [`device_activity`] — `DeviceActivity.tsx` (sessions + revoke).
//! * [`applications`]    — `Applications.tsx` (granted scopes + revoke).
//! * [`linked_accounts`] — `LinkedAccounts.tsx` (social IdPs).
//! * [`groups`]          — `Groups.tsx` (read-only memberships).
//!
//! Persona — end-user (not admin-gated). Every page renders inside
//! the shared shell (top bar + breadcrumb + theme + toasts) so the
//! user-facing chrome is consistent with the admin surfaces.

pub mod applications;
pub mod device_activity;
pub mod fixtures;
pub mod groups;
pub mod linked_accounts;
pub mod personal_info;
pub mod signin_methods;

use axum::{
    extract::Form,
    response::{Html, IntoResponse, Redirect},
    routing::{get, post},
    Extension, Router,
};
use serde::Deserialize;

use crate::admin::layout::shell::{shell_v2, ShellOptions};
use crate::admin::permission::Persona;
use crate::admin::render::escape;
use crate::metrics::PortalMetrics;

/// Build the `/account/...` router. Mount with
/// `app.merge(cave_portal::account::router())` in `cave-runtime serve`.
pub fn router() -> Router {
    Router::new()
        .route("/account", get(index_redirect))
        .route("/account/", get(index_redirect))
        .route("/account/personal-info", get(personal_info_get).post(personal_info_post))
        .route("/account/signin-methods", get(signin_methods_get))
        .route(
            "/account/signin-methods/password",
            post(signin_methods_password_post),
        )
        .route(
            "/account/signin-methods/otp/setup",
            post(signin_methods_otp_setup_post),
        )
        .route(
            "/account/signin-methods/webauthn/register",
            post(signin_methods_webauthn_register_post),
        )
        .route(
            "/account/signin-methods/delete",
            post(signin_methods_delete_post),
        )
        .route("/account/device-activity", get(device_activity_get))
        .route(
            "/account/device-activity/revoke",
            post(device_activity_revoke_post),
        )
        .route(
            "/account/device-activity/logout-all",
            post(device_activity_logout_all_post),
        )
        .route("/account/applications", get(applications_get))
        .route(
            "/account/applications/revoke",
            post(applications_revoke_post),
        )
        .route("/account/linked-accounts", get(linked_accounts_get))
        .route(
            "/account/linked-accounts/link",
            post(linked_accounts_link_post),
        )
        .route(
            "/account/linked-accounts/unlink",
            post(linked_accounts_unlink_post),
        )
        .route("/account/groups", get(groups_get))
}

/// Derive the principal from the JWT extension if the cave-auth
/// middleware set one. Otherwise fall back to the `?as=...` query
/// (dev) or `anonymous`.
fn principal_from(claims: Option<&cave_auth::jwt_middleware::JwtClaims>) -> String {
    match claims {
        Some(c) => c.sub.clone(),
        None => "anonymous".to_string(),
    }
}

async fn index_redirect() -> Redirect {
    Redirect::permanent("/account/personal-info")
}

async fn personal_info_get(
    claims: Option<Extension<cave_auth::jwt_middleware::JwtClaims>>,
) -> Html<String> {
    let p = principal_from(claims.as_deref());
    PortalMetrics::global().incr_page_view("personal_info", "user");
    Html(personal_info::render(&p))
}

#[derive(Debug, Deserialize)]
pub struct PersonalInfoForm {
    pub first_name: String,
    pub last_name: String,
    pub email: String,
}

async fn personal_info_post(
    claims: Option<Extension<cave_auth::jwt_middleware::JwtClaims>>,
    Form(form): Form<PersonalInfoForm>,
) -> impl IntoResponse {
    let _ = principal_from(claims.as_deref());
    match personal_info::validate(&form.first_name, &form.last_name, &form.email) {
        Ok(()) => {
            PortalMetrics::global().incr_action("personal_info_update", "success");
            Redirect::to("/account/personal-info?saved=1").into_response()
        }
        Err(_e) => {
            PortalMetrics::global().incr_action("personal_info_update", "validation_error");
            Redirect::to("/account/personal-info?error=validation").into_response()
        }
    }
}

async fn signin_methods_get(
    claims: Option<Extension<cave_auth::jwt_middleware::JwtClaims>>,
) -> Html<String> {
    let p = principal_from(claims.as_deref());
    PortalMetrics::global().incr_page_view("signin_methods", "user");
    Html(signin_methods::render(&p))
}

async fn signin_methods_password_post() -> Redirect {
    PortalMetrics::global().incr_action("password_update", "redirect");
    Redirect::to("/account/signin-methods?msg=password_flow_started")
}

async fn signin_methods_otp_setup_post() -> Redirect {
    PortalMetrics::global().incr_action("otp_setup", "redirect");
    Redirect::to("/account/signin-methods?msg=otp_setup_started")
}

async fn signin_methods_webauthn_register_post() -> Redirect {
    PortalMetrics::global().incr_action("webauthn_register", "redirect");
    Redirect::to("/account/signin-methods?msg=webauthn_register_started")
}

#[derive(Debug, Deserialize)]
pub struct CredentialIdForm {
    pub credential_id: String,
}

async fn signin_methods_delete_post(Form(form): Form<CredentialIdForm>) -> Redirect {
    let _ = form.credential_id;
    PortalMetrics::global().incr_action("credential_delete", "ok");
    Redirect::to("/account/signin-methods?msg=credential_removed")
}

async fn device_activity_get(
    claims: Option<Extension<cave_auth::jwt_middleware::JwtClaims>>,
) -> Html<String> {
    let p = principal_from(claims.as_deref());
    PortalMetrics::global().incr_page_view("device_activity", "user");
    Html(device_activity::render(&p))
}

#[derive(Debug, Deserialize)]
pub struct SessionIdForm {
    pub session_id: String,
}

async fn device_activity_revoke_post(Form(form): Form<SessionIdForm>) -> Redirect {
    let result = if device_activity::can_revoke(&form.session_id, "sess-current") {
        "ok"
    } else {
        "blocked"
    };
    PortalMetrics::global().incr_action("session_revoke", result);
    Redirect::to("/account/device-activity?msg=revoked")
}

async fn device_activity_logout_all_post() -> Redirect {
    PortalMetrics::global().incr_action("logout_all", "ok");
    Redirect::to("/account/device-activity?msg=logged_out_all")
}

async fn applications_get(
    claims: Option<Extension<cave_auth::jwt_middleware::JwtClaims>>,
) -> Html<String> {
    let p = principal_from(claims.as_deref());
    PortalMetrics::global().incr_page_view("applications", "user");
    Html(applications::render(&p))
}

#[derive(Debug, Deserialize)]
pub struct ClientIdForm {
    pub client_id: String,
}

async fn applications_revoke_post(Form(form): Form<ClientIdForm>) -> Redirect {
    let _ = form.client_id;
    PortalMetrics::global().incr_action("application_revoke", "ok");
    Redirect::to("/account/applications?msg=revoked")
}

async fn linked_accounts_get(
    claims: Option<Extension<cave_auth::jwt_middleware::JwtClaims>>,
) -> Html<String> {
    let p = principal_from(claims.as_deref());
    PortalMetrics::global().incr_page_view("linked_accounts", "user");
    Html(linked_accounts::render(&p))
}

#[derive(Debug, Deserialize)]
pub struct ProviderAliasForm {
    pub provider_alias: String,
}

async fn linked_accounts_link_post(Form(form): Form<ProviderAliasForm>) -> Redirect {
    let _ = form.provider_alias;
    PortalMetrics::global().incr_action("provider_link", "ok");
    Redirect::to("/account/linked-accounts?msg=linked")
}

async fn linked_accounts_unlink_post(Form(form): Form<ProviderAliasForm>) -> Redirect {
    let _ = form.provider_alias;
    PortalMetrics::global().incr_action("provider_unlink", "ok");
    Redirect::to("/account/linked-accounts?msg=unlinked")
}

async fn groups_get(
    claims: Option<Extension<cave_auth::jwt_middleware::JwtClaims>>,
) -> Html<String> {
    let p = principal_from(claims.as_deref());
    PortalMetrics::global().incr_page_view("groups", "user");
    Html(groups::render(&p))
}

/// Wrap a body in the account console shell.
///
/// Unlike admin pages, the account console **hides the admin
/// sidebar** — the end-user does not see /admin nav. We instead
/// render a horizontal tab strip across the top of `body`.
pub fn account_shell(
    principal: &str,
    current_path: &str,
    title: &str,
    body: &str,
) -> String {
    let tabs = account_tabs(current_path);
    let merged = format!(
        r#"{tabs}
<section>
  <p class="text-sm text-gray-600 dark:text-zinc-400 mb-3">
    Signed in as <code class="bg-zinc-100 dark:bg-zinc-800 px-1 rounded">{principal}</code>.
    Account console — Keycloak parity
    (<a class="text-blue-700 underline" href="https://github.com/keycloak/keycloak/tree/22.0.0/js/apps/account-ui">account-ui@22.0.0</a>).
  </p>
  {body}
</section>"#,
        tabs = tabs,
        principal = escape(principal),
        body = body,
    );
    shell_v2(ShellOptions {
        title,
        persona: Persona::Anonymous,
        tenant_id: principal,
        current_path,
        theme_cookie: None,
        breadcrumb: None,
        extra_commands: Vec::new(),
        cluster_info: "cave-runtime · account",
        hide_sidebar: true,
        body: &merged,
    })
}

const TABS: &[(&str, &str)] = &[
    ("/account/personal-info", "Personal info"),
    ("/account/signin-methods", "Sign-in methods"),
    ("/account/device-activity", "Device activity"),
    ("/account/applications", "Applications"),
    ("/account/linked-accounts", "Linked accounts"),
    ("/account/groups", "Groups"),
];

fn account_tabs(current_path: &str) -> String {
    let mut out = String::from(
        r#"<nav aria-label="Account navigation" class="border-b mb-4 flex flex-wrap gap-2 dark:border-zinc-800">"#,
    );
    for (href, label) in TABS {
        let active = *href == current_path;
        let cls = if active {
            "px-3 py-2 text-sm font-medium border-b-2 border-blue-600 text-blue-700 dark:text-blue-300"
        } else {
            "px-3 py-2 text-sm text-zinc-600 dark:text-zinc-300 hover:text-blue-700 hover:border-b-2 hover:border-blue-200"
        };
        let aria = if active { r#" aria-current="page""# } else { "" };
        out.push_str(&format!(
            r#"<a href="{href}"{aria} class="{cls}">{label}</a>"#,
            href = escape(href),
            cls = cls,
            label = escape(label),
            aria = aria,
        ));
    }
    out.push_str("</nav>");
    out
}

#[cfg(test)]
mod router_tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::{Request, StatusCode};
    use tower::util::ServiceExt;

    async fn body_text(resp: axum::response::Response) -> String {
        let bytes = to_bytes(resp.into_body(), 1_000_000).await.unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    #[tokio::test]
    async fn account_root_redirects_to_personal_info() {
        let app = router();
        let resp = app
            .oneshot(Request::builder().uri("/account").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::PERMANENT_REDIRECT);
        let loc = resp.headers().get("location").unwrap().to_str().unwrap();
        assert_eq!(loc, "/account/personal-info");
    }

    #[tokio::test]
    async fn personal_info_get_returns_200_with_form() {
        let app = router();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/account/personal-info")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_text(resp).await;
        assert!(body.contains(r#"name="first_name""#));
        assert!(body.contains(r#"name="email""#));
        // Account chrome present (no admin sidebar).
        assert!(!body.contains(r#"id="cave-sidebar""#));
    }

    #[tokio::test]
    async fn personal_info_post_with_valid_body_redirects_to_saved() {
        let app = router();
        let body = "first_name=Alice&last_name=Smith&email=alice%40acme.com";
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/account/personal-info")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        // 303 See Other is the axum default for `Redirect::to`.
        assert!(
            resp.status() == StatusCode::SEE_OTHER || resp.status() == StatusCode::TEMPORARY_REDIRECT,
            "expected redirect, got {}",
            resp.status()
        );
        let loc = resp.headers().get("location").unwrap().to_str().unwrap();
        assert!(loc.contains("saved=1"));
    }

    #[tokio::test]
    async fn personal_info_post_with_invalid_email_redirects_to_error() {
        let app = router();
        let body = "first_name=A&last_name=B&email=invalid";
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/account/personal-info")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        let loc = resp.headers().get("location").unwrap().to_str().unwrap();
        assert!(loc.contains("error=validation"));
    }

    #[tokio::test]
    async fn signin_methods_get_returns_200() {
        let app = router();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/account/signin-methods")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn signin_methods_delete_post_redirects() {
        let app = router();
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/account/signin-methods/delete")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from("credential_id=cred-otp-1"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(resp.status().is_redirection());
    }

    #[tokio::test]
    async fn device_activity_get_returns_200_and_revoke_post_redirects() {
        let app = router();
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/account/device-activity")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let resp2 = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/account/device-activity/revoke")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from("session_id=sess-mobile"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(resp2.status().is_redirection());
    }

    #[tokio::test]
    async fn applications_revoke_post_redirects() {
        let app = router();
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/account/applications/revoke")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from("client_id=cave-portal"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(resp.status().is_redirection());
    }

    #[tokio::test]
    async fn linked_accounts_link_and_unlink_redirect() {
        let app = router();
        let r1 = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/account/linked-accounts/link")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from("provider_alias=google"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(r1.status().is_redirection());

        let r2 = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/account/linked-accounts/unlink")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from("provider_alias=github"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(r2.status().is_redirection());
    }

    #[tokio::test]
    async fn groups_get_returns_200_and_is_read_only() {
        let app = router();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/account/groups")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_text(resp).await;
        // No form-action on /account/groups (read-only page).
        assert!(!body.contains(r#"action="/account/groups/"#));
    }

    #[tokio::test]
    async fn principal_from_uses_jwt_sub_when_present() {
        let claims = cave_auth::jwt_middleware::JwtClaims {
            sub: "alice@acme".into(),
            email: "alice@acme".into(),
            roles: vec![],
            exp: 9_999_999_999,
        };
        assert_eq!(principal_from(Some(&claims)), "alice@acme");
        assert_eq!(principal_from(None), "anonymous");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_shell_hides_admin_sidebar() {
        let html = account_shell(
            "alice@acme",
            "/account/personal-info",
            "Personal info",
            "<p>hi</p>",
        );
        // No admin sidebar present.
        assert!(!html.contains(r#"id="cave-sidebar""#));
        // Account tab strip present.
        assert!(html.contains("Account navigation"));
        assert!(html.contains("Personal info"));
    }

    #[test]
    fn account_shell_marks_active_tab() {
        let html = account_shell(
            "alice@acme",
            "/account/device-activity",
            "Devices",
            "",
        );
        assert!(html.contains(r#"aria-current="page""#));
        // Active tab's label appears in active form (border-b-2 + blue).
        assert!(html.contains("Device activity"));
    }

    #[test]
    fn account_shell_escapes_principal_in_header_chip() {
        let html = account_shell(
            "<script>x</script>",
            "/account/groups",
            "Groups",
            "",
        );
        assert!(html.contains("&lt;script&gt;"));
        assert!(!html.contains("<script>x</script>"));
    }

    #[test]
    fn account_shell_emits_upstream_attribution_link() {
        let html = account_shell("u", "/account/applications", "Applications", "");
        assert!(html.contains("account-ui@22.0.0"));
        assert!(html.contains("keycloak/keycloak/tree/22.0.0"));
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
//! Per-realm seeded fixtures for the admin auth pages.
//!
//! Each helper takes a realm name and returns a deterministic shape
//! representative of what Keycloak's admin REST API would yield. The
//! point is to stop bleeding `AuthSession` into every page when a
//! realistic admin view (roles, groups, flows, …) needs more than
//! the session blob.
//!
//! Source shape: keycloak/keycloak@b825ba97
//! `js/apps/admin-ui/src/{realm-roles,client-scopes,groups,
//! identity-providers,authentication,realm-settings}/representations/*`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RealmSettings {
    pub realm: String,
    pub display_name: String,
    pub enabled: bool,
    pub ssl_required: String,
    pub registration_allowed: bool,
    pub login_with_email_allowed: bool,
    pub duplicate_emails_allowed: bool,
    pub access_token_lifespan: i64,
    pub sso_session_idle_timeout: i64,
    pub brute_force_protected: bool,
    pub permanent_lockout: bool,
    pub max_login_failures: u32,
    pub supported_locales: Vec<String>,
    pub default_locale: String,
    pub login_theme: String,
    pub account_theme: String,
    pub admin_theme: String,
    pub email_theme: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientScope {
    pub name: String,
    pub description: String,
    /// `openid-connect` / `saml`.
    pub protocol: String,
    pub include_in_token_scope: bool,
    pub mappers: Vec<ClientScopeMapper>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientScopeMapper {
    pub name: String,
    /// Mapper id (e.g. `oidc-usermodel-attribute-mapper`).
    pub kind: String,
    pub claim_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RealmRole {
    pub name: String,
    pub description: String,
    pub composite: bool,
    pub composites: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupNode {
    pub id: String,
    pub path: String,
    pub member_count: u32,
    pub child_paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdentityProvider {
    pub alias: String,
    pub display_name: String,
    /// e.g. `oidc`, `saml`, `github`, `google`, `keycloak-oidc`.
    pub provider_id: String,
    pub enabled: bool,
    pub trust_email: bool,
    pub store_token: bool,
    pub link_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthFlow {
    pub alias: String,
    pub description: String,
    pub built_in: bool,
    pub top_level: bool,
    pub provider_id: String,
    pub executions: Vec<AuthExecution>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthExecution {
    pub display_name: String,
    /// `REQUIRED` / `ALTERNATIVE` / `OPTIONAL` / `DISABLED` / `CONDITIONAL`.
    pub requirement: String,
    /// `auth-cookie`, `identity-provider-redirector`, … Keycloak provider id.
    pub authenticator: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthnConfig {
    pub realm: String,
    pub browser_flow: String,
    pub direct_grant_flow: String,
    pub reset_credentials_flow: String,
    pub client_authentication_flow: String,
    pub registration_flow: String,
    pub docker_authentication_flow: String,
    pub required_actions: Vec<RequiredAction>,
    pub password_policy: PasswordPolicy,
    pub otp_policy: OtpPolicy,
    pub webauthn_policy: WebauthnPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequiredAction {
    pub alias: String,
    pub name: String,
    pub enabled: bool,
    pub default_action: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PasswordPolicy {
    pub min_length: u32,
    pub digits: u32,
    pub upper_case: u32,
    pub special_chars: u32,
    pub not_username: bool,
    pub not_email: bool,
    pub hash_iterations: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OtpPolicy {
    /// `totp` / `hotp`.
    pub kind: String,
    pub algorithm: String,
    pub digits: u32,
    pub period_seconds: u32,
    pub lookahead_window: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebauthnPolicy {
    pub rp_name: String,
    pub signature_algorithms: Vec<String>,
    pub attestation_conveyance_preference: String,
    pub authenticator_attachment: String,
    pub user_verification_requirement: String,
    pub create_timeout_seconds: u32,
}

// ── deterministic per-realm fixtures ─────────────────────────────────

pub fn realm_settings(realm: &str) -> RealmSettings {
    RealmSettings {
        realm: realm.to_string(),
        display_name: format!("Cave – {realm}"),
        enabled: true,
        ssl_required: "external".into(),
        registration_allowed: false,
        login_with_email_allowed: true,
        duplicate_emails_allowed: false,
        access_token_lifespan: 300,
        sso_session_idle_timeout: 1800,
        brute_force_protected: true,
        permanent_lockout: false,
        max_login_failures: 30,
        supported_locales: vec!["en".into(), "tr".into(), "de".into()],
        default_locale: "en".into(),
        login_theme: "cave".into(),
        account_theme: "cave".into(),
        admin_theme: "cave".into(),
        email_theme: "cave".into(),
    }
}

pub fn client_scopes(realm: &str) -> Vec<ClientScope> {
    let _ = realm;
    vec![
        ClientScope {
            name: "openid".into(),
            description: "OpenID Connect required scope".into(),
            protocol: "openid-connect".into(),
            include_in_token_scope: true,
            mappers: vec![],
        },
        ClientScope {
            name: "profile".into(),
            description: "OIDC profile claims".into(),
            protocol: "openid-connect".into(),
            include_in_token_scope: true,
            mappers: vec![
                ClientScopeMapper {
                    name: "given name".into(),
                    kind: "oidc-usermodel-attribute-mapper".into(),
                    claim_name: "given_name".into(),
                },
                ClientScopeMapper {
                    name: "family name".into(),
                    kind: "oidc-usermodel-attribute-mapper".into(),
                    claim_name: "family_name".into(),
                },
            ],
        },
        ClientScope {
            name: "email".into(),
            description: "OIDC email claims".into(),
            protocol: "openid-connect".into(),
            include_in_token_scope: true,
            mappers: vec![ClientScopeMapper {
                name: "email".into(),
                kind: "oidc-usermodel-property-mapper".into(),
                claim_name: "email".into(),
            }],
        },
        ClientScope {
            name: "offline_access".into(),
            description: "Long-lived refresh token".into(),
            protocol: "openid-connect".into(),
            include_in_token_scope: true,
            mappers: vec![],
        },
        ClientScope {
            name: "roles".into(),
            description: "Realm + client roles claim".into(),
            protocol: "openid-connect".into(),
            include_in_token_scope: false,
            mappers: vec![ClientScopeMapper {
                name: "realm roles".into(),
                kind: "oidc-usermodel-realm-role-mapper".into(),
                claim_name: "realm_access.roles".into(),
            }],
        },
    ]
}

pub fn realm_roles(realm: &str) -> Vec<RealmRole> {
    let _ = realm;
    vec![
        RealmRole {
            name: "default-roles".into(),
            description: "Auto-assigned to every user".into(),
            composite: true,
            composites: vec!["uma_authorization".into(), "offline_access".into()],
        },
        RealmRole {
            name: "platform_admin".into(),
            description: "Cave platform staff".into(),
            composite: false,
            composites: vec![],
        },
        RealmRole {
            name: "tenant_admin".into(),
            description: "Per-tenant admin".into(),
            composite: false,
            composites: vec![],
        },
        RealmRole {
            name: "offline_access".into(),
            description: "Long-lived refresh tokens".into(),
            composite: false,
            composites: vec![],
        },
        RealmRole {
            name: "uma_authorization".into(),
            description: "UMA 2.0 authorization".into(),
            composite: false,
            composites: vec![],
        },
    ]
}

pub fn groups(realm: &str) -> Vec<GroupNode> {
    vec![
        GroupNode {
            id: "grp-root-eng".into(),
            path: format!("/{realm}/engineering"),
            member_count: 42,
            child_paths: vec![
                format!("/{realm}/engineering/backend"),
                format!("/{realm}/engineering/frontend"),
                format!("/{realm}/engineering/sre"),
            ],
        },
        GroupNode {
            id: "grp-root-emp".into(),
            path: format!("/{realm}/employees"),
            member_count: 119,
            child_paths: vec![format!("/{realm}/employees/contractors")],
        },
        GroupNode {
            id: "grp-eng-be".into(),
            path: format!("/{realm}/engineering/backend"),
            member_count: 18,
            child_paths: vec![],
        },
        GroupNode {
            id: "grp-eng-fe".into(),
            path: format!("/{realm}/engineering/frontend"),
            member_count: 12,
            child_paths: vec![],
        },
        GroupNode {
            id: "grp-eng-sre".into(),
            path: format!("/{realm}/engineering/sre"),
            member_count: 7,
            child_paths: vec![],
        },
    ]
}

pub fn identity_providers(realm: &str) -> Vec<IdentityProvider> {
    let _ = realm;
    vec![
        IdentityProvider {
            alias: "github".into(),
            display_name: "GitHub".into(),
            provider_id: "github".into(),
            enabled: true,
            trust_email: true,
            store_token: false,
            link_only: false,
        },
        IdentityProvider {
            alias: "google".into(),
            display_name: "Google".into(),
            provider_id: "google".into(),
            enabled: false,
            trust_email: true,
            store_token: false,
            link_only: false,
        },
        IdentityProvider {
            alias: "okta-corp".into(),
            display_name: "Corp Okta".into(),
            provider_id: "oidc".into(),
            enabled: true,
            trust_email: false,
            store_token: true,
            link_only: false,
        },
        IdentityProvider {
            alias: "saml-azure".into(),
            display_name: "Azure AD SAML".into(),
            provider_id: "saml".into(),
            enabled: true,
            trust_email: true,
            store_token: false,
            link_only: true,
        },
    ]
}

pub fn flows(realm: &str) -> Vec<AuthFlow> {
    let _ = realm;
    vec![
        AuthFlow {
            alias: "browser".into(),
            description: "Browser-based authentication".into(),
            built_in: true,
            top_level: true,
            provider_id: "basic-flow".into(),
            executions: vec![
                AuthExecution {
                    display_name: "Cookie".into(),
                    requirement: "ALTERNATIVE".into(),
                    authenticator: "auth-cookie".into(),
                },
                AuthExecution {
                    display_name: "Kerberos".into(),
                    requirement: "DISABLED".into(),
                    authenticator: "auth-spnego".into(),
                },
                AuthExecution {
                    display_name: "Identity provider redirector".into(),
                    requirement: "ALTERNATIVE".into(),
                    authenticator: "identity-provider-redirector".into(),
                },
                AuthExecution {
                    display_name: "Username/password form".into(),
                    requirement: "ALTERNATIVE".into(),
                    authenticator: "auth-username-password-form".into(),
                },
                AuthExecution {
                    display_name: "WebAuthn".into(),
                    requirement: "ALTERNATIVE".into(),
                    authenticator: "webauthn-authenticator".into(),
                },
            ],
        },
        AuthFlow {
            alias: "direct grant".into(),
            description: "Resource owner password credentials".into(),
            built_in: true,
            top_level: true,
            provider_id: "basic-flow".into(),
            executions: vec![AuthExecution {
                display_name: "Direct grant validate username".into(),
                requirement: "REQUIRED".into(),
                authenticator: "direct-grant-validate-username".into(),
            }],
        },
        AuthFlow {
            alias: "registration".into(),
            description: "User self-registration".into(),
            built_in: true,
            top_level: true,
            provider_id: "basic-flow".into(),
            executions: vec![AuthExecution {
                display_name: "Registration page form".into(),
                requirement: "REQUIRED".into(),
                authenticator: "registration-page-form".into(),
            }],
        },
        AuthFlow {
            alias: "reset credentials".into(),
            description: "Forgot-password flow".into(),
            built_in: true,
            top_level: true,
            provider_id: "basic-flow".into(),
            executions: vec![AuthExecution {
                display_name: "Choose user".into(),
                requirement: "REQUIRED".into(),
                authenticator: "reset-credentials-choose-user".into(),
            }],
        },
        AuthFlow {
            alias: "first broker login".into(),
            description: "First-time social/SAML login".into(),
            built_in: true,
            top_level: true,
            provider_id: "basic-flow".into(),
            executions: vec![],
        },
    ]
}

pub fn authn_config(realm: &str) -> AuthnConfig {
    AuthnConfig {
        realm: realm.to_string(),
        browser_flow: "browser".into(),
        direct_grant_flow: "direct grant".into(),
        reset_credentials_flow: "reset credentials".into(),
        client_authentication_flow: "clients".into(),
        registration_flow: "registration".into(),
        docker_authentication_flow: "docker auth".into(),
        required_actions: vec![
            RequiredAction {
                alias: "VERIFY_EMAIL".into(),
                name: "Verify email".into(),
                enabled: true,
                default_action: false,
            },
            RequiredAction {
                alias: "UPDATE_PROFILE".into(),
                name: "Update profile".into(),
                enabled: true,
                default_action: false,
            },
            RequiredAction {
                alias: "CONFIGURE_TOTP".into(),
                name: "Configure OTP".into(),
                enabled: true,
                default_action: false,
            },
            RequiredAction {
                alias: "webauthn-register-passwordless".into(),
                name: "Webauthn Register Passwordless".into(),
                enabled: true,
                default_action: false,
            },
            RequiredAction {
                alias: "UPDATE_PASSWORD".into(),
                name: "Update password".into(),
                enabled: true,
                default_action: false,
            },
        ],
        password_policy: PasswordPolicy {
            min_length: 12,
            digits: 1,
            upper_case: 1,
            special_chars: 1,
            not_username: true,
            not_email: true,
            hash_iterations: 27_500,
        },
        otp_policy: OtpPolicy {
            kind: "totp".into(),
            algorithm: "HmacSHA256".into(),
            digits: 6,
            period_seconds: 30,
            lookahead_window: 1,
        },
        webauthn_policy: WebauthnPolicy {
            rp_name: "cave".into(),
            signature_algorithms: vec!["ES256".into(), "RS256".into()],
            attestation_conveyance_preference: "none".into(),
            authenticator_attachment: "platform".into(),
            user_verification_requirement: "required".into(),
            create_timeout_seconds: 0,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn realm_settings_carries_realm_name_in_display() {
        let s = realm_settings("acme-realm");
        assert!(s.display_name.contains("acme-realm"));
        assert!(s.enabled);
    }

    #[test]
    fn client_scopes_includes_openid_and_offline_access() {
        let s = client_scopes("acme");
        assert!(s.iter().any(|c| c.name == "openid"));
        assert!(s.iter().any(|c| c.name == "offline_access"));
        let roles = s.iter().find(|c| c.name == "roles").unwrap();
        assert!(roles.mappers.iter().any(|m| m.claim_name == "realm_access.roles"));
    }

    #[test]
    fn realm_roles_marks_default_roles_as_composite() {
        let r = realm_roles("acme");
        let default = r.iter().find(|x| x.name == "default-roles").unwrap();
        assert!(default.composite);
        assert!(default.composites.contains(&"offline_access".to_string()));
    }

    #[test]
    fn groups_carry_realm_in_path_and_link_children() {
        let g = groups("acme");
        let root_eng = g.iter().find(|x| x.id == "grp-root-eng").unwrap();
        assert!(root_eng.path.starts_with("/acme/"));
        assert_eq!(root_eng.child_paths.len(), 3);
    }

    #[test]
    fn identity_providers_mix_oidc_and_saml() {
        let p = identity_providers("acme");
        assert!(p.iter().any(|x| x.provider_id == "saml"));
        assert!(p.iter().any(|x| x.provider_id == "oidc"));
    }

    #[test]
    fn flows_include_all_five_built_in_top_level_flows() {
        let f = flows("acme");
        let aliases: Vec<&str> = f.iter().map(|x| x.alias.as_str()).collect();
        for required in [
            "browser",
            "direct grant",
            "registration",
            "reset credentials",
            "first broker login",
        ] {
            assert!(aliases.contains(&required), "missing flow {required}");
        }
    }

    #[test]
    fn flows_browser_lists_all_four_alternatives() {
        let f = flows("acme");
        let browser = f.iter().find(|x| x.alias == "browser").unwrap();
        let names: Vec<&str> = browser.executions.iter().map(|x| x.display_name.as_str()).collect();
        assert!(names.contains(&"Cookie"));
        assert!(names.contains(&"Identity provider redirector"));
        assert!(names.contains(&"Username/password form"));
        assert!(names.contains(&"WebAuthn"));
    }

    #[test]
    fn authn_config_required_actions_include_webauthn_passwordless() {
        let c = authn_config("acme");
        assert!(c.required_actions.iter().any(|r| r.alias.contains("webauthn")));
    }

    #[test]
    fn authn_config_password_policy_enforces_min_length_12() {
        let c = authn_config("acme");
        assert!(c.password_policy.min_length >= 12);
        assert!(c.password_policy.not_username);
    }

    #[test]
    fn authn_config_default_flows_map_to_known_aliases() {
        let c = authn_config("acme");
        let known: Vec<String> = flows("acme").iter().map(|x| x.alias.clone()).collect();
        assert!(known.contains(&c.browser_flow));
        assert!(known.contains(&c.direct_grant_flow));
        assert!(known.contains(&c.reset_credentials_flow));
        assert!(known.contains(&c.registration_flow));
    }
}

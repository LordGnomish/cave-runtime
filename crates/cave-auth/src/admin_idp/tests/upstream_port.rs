// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 testsuite/integration-arquillian/.../admin/IdentityProviderTest.java
//
//! Upstream-port traceability tests for the identity-provider admin REST.

use super::super::{AdminIdpState, instances::IdentityProvider, mappers::IdentityProviderMapper};
use crate::keycloak::realm::{RealmRequest, RealmStore};
use chrono::Utc;

async fn fresh_state() -> AdminIdpState {
    let realms = RealmStore::new();
    realms.create(RealmRequest { id: "r".into(), display_name: None, enabled: None, ssl_required: None, registration_allowed: None, login_with_email_allowed: None, duplicate_emails_allowed: None, access_token_lifespan: None, sso_session_idle_timeout: None }).await.unwrap();
    AdminIdpState::new(realms)
}

// upstream: keycloak/keycloak IdentityProviderTest.java:storeRoundtrip
#[tokio::test]
async fn store_roundtrip() {
    let state = fresh_state().await;
    state.providers.create("r", IdentityProvider {
        alias: "oidc1".into(), display_name: Some("OIDC One".into()),
        provider_id: "oidc".into(), enabled: true,
        config: Default::default(), created_at: Utc::now(),
    }).await.unwrap();
    assert_eq!(state.providers.count("r").await, 1);
    let got = state.providers.get("r", "oidc1").await.unwrap();
    assert_eq!(got.display_name.as_deref(), Some("OIDC One"));
}

// upstream: keycloak/keycloak IdentityProviderTest.java:listMappersForUnknownAliasIsEmpty
#[tokio::test]
async fn list_mappers_for_unknown_alias_is_empty() {
    let state = fresh_state().await;
    assert!(state.mappers.list("r", "ghost").await.is_empty());
}

// upstream: keycloak/keycloak IdentityProviderTest.java:mapperStoredAndRetrieved
#[tokio::test]
async fn mapper_stored_and_retrieved() {
    let state = fresh_state().await;
    state.providers.create("r", IdentityProvider {
        alias: "g".into(), display_name: None, provider_id: "github".into(),
        enabled: true, config: Default::default(), created_at: Utc::now(),
    }).await.unwrap();
    let m = IdentityProviderMapper {
        id: "mid".into(), name: "email".into(),
        identity_provider_mapper: "oidc-user-attribute-idp-mapper".into(),
        config: Default::default(),
    };
    state.mappers.create("r", "g", m.clone()).await.unwrap();
    assert_eq!(state.mappers.get("r", "g", "mid").await.unwrap(), m);
}

// upstream: keycloak/keycloak IdentityProviderTest.java:duplicateMapperIdRejected
#[tokio::test]
async fn duplicate_mapper_id_rejected() {
    let state = fresh_state().await;
    state.providers.create("r", IdentityProvider {
        alias: "g".into(), display_name: None, provider_id: "github".into(),
        enabled: true, config: Default::default(), created_at: Utc::now(),
    }).await.unwrap();
    let m = IdentityProviderMapper {
        id: "mid".into(), name: "n".into(),
        identity_provider_mapper: "oidc-role-idp-mapper".into(),
        config: Default::default(),
    };
    state.mappers.create("r", "g", m.clone()).await.unwrap();
    assert!(state.mappers.create("r", "g", m).await.is_err());
}

// SPDX-License-Identifier: AGPL-3.0-or-later
//! Portal → cave-auth live data source — RED phase.
//!
//! Only the trait + DTOs + stub impls exist here in this commit; every
//! impl method returns `ClientError::NotWired`. The companion test
//! module enumerates the contract the GREEN commit must satisfy.
//!
//! Source: keycloak/keycloak@v22.0.0
//!         services/src/main/java/org/keycloak/services/resources/admin/AdminRoot.java

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("cave-auth request failed: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("cave-auth returned status {status}: {body}")]
    Status {
        status: reqwest::StatusCode,
        body: String,
    },
    #[error("cave-auth response parse failed: {0}")]
    Decode(String),
    #[error("resource {0} is not wired against the live cave-auth yet")]
    NotWired(&'static str),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("not found: {0}")]
    NotFound(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Realm {
    pub id: String,
    #[serde(default, rename = "displayName")]
    pub display_name: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default, rename = "sslRequired")]
    pub ssl_required: String,
    #[serde(default, rename = "accessTokenLifespan")]
    pub access_token_lifespan: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ClientApp {
    pub id: String,
    #[serde(rename = "clientId")]
    pub client_id: String,
    #[serde(default)]
    pub realm: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct User {
    pub id: String,
    #[serde(default)]
    pub realm: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct EventPayload {
    pub time: i64,
    pub realm: String,
    #[serde(rename = "type")]
    pub kind: String,
}

#[async_trait]
pub trait AuthClient: Send + Sync + std::fmt::Debug {
    async fn list_realms(&self) -> Result<Vec<Realm>, ClientError>;
    async fn create_realm(&self, realm: &Realm) -> Result<Realm, ClientError>;
}

/// Stub mock — every method returns `NotWired` until the GREEN commit
/// fills in the real backing store.
#[derive(Debug, Default)]
pub struct AuthMockClient {
    _realms: RwLock<HashMap<String, Realm>>,
}

impl AuthMockClient {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl AuthClient for AuthMockClient {
    async fn list_realms(&self) -> Result<Vec<Realm>, ClientError> {
        Err(ClientError::NotWired("list_realms"))
    }
    async fn create_realm(&self, _realm: &Realm) -> Result<Realm, ClientError> {
        Err(ClientError::NotWired("create_realm"))
    }
}

#[derive(Debug)]
pub struct AuthApiClient {
    _client: reqwest::Client,
    _base_url: String,
}

impl AuthApiClient {
    #[allow(dead_code)]
    pub fn test_against(base_url: String) -> Self {
        Self {
            _client: reqwest::Client::builder()
                .timeout(Duration::from_secs(2))
                .build()
                .expect("test client"),
            _base_url: base_url,
        }
    }
}

#[async_trait]
impl AuthClient for AuthApiClient {
    async fn list_realms(&self) -> Result<Vec<Realm>, ClientError> {
        Err(ClientError::NotWired("list_realms"))
    }
    async fn create_realm(&self, _realm: &Realm) -> Result<Realm, ClientError> {
        Err(ClientError::NotWired("create_realm"))
    }
}

pub type SharedAuthClient = Arc<dyn AuthClient>;

#[cfg(test)]
mod tests {
    use super::*;

    fn realm_acme() -> Realm {
        Realm {
            id: "acme-realm".into(),
            display_name: "Acme".into(),
            enabled: true,
            ssl_required: "external".into(),
            access_token_lifespan: 300,
        }
    }

    // RED: these will fail because the stub impl returns NotWired.
    // The GREEN commit replaces the stub with a real in-memory store
    // and a reqwest-driven client.

    #[tokio::test]
    async fn mock_create_realm_persists() {
        let c = AuthMockClient::new();
        let r = c.create_realm(&realm_acme()).await.unwrap();
        assert_eq!(r.id, "acme-realm");
        let listed = c.list_realms().await.unwrap();
        assert_eq!(listed.len(), 1);
    }
}

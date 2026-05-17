// SPDX-License-Identifier: AGPL-3.0-or-later
//! Portal → cave-auth live data source.
//!
//! `AuthClient` is the typed seam Portal admin handlers can read from
//! instead of `AdminState`'s seeded fixtures. Implementations:
//!
//! * [`AuthApiClient`] — talks to a cave-auth admin REST endpoint
//!   over HTTPS via `reqwest` with the same CA-pinned mTLS surface
//!   as the apiserver wiring (`admin::runtime_client::ApiserverClient`).
//! * [`AuthMockClient`] — in-memory backing store. Used by handler
//!   tests and the `?tenant_id=...` dev smoke flow when no real
//!   cave-auth is wired up.
//!
//! Method shape mirrors the Keycloak admin REST resource:
//! `/admin/realms` for realms, `/admin/realms/{realm}/{resource}` for
//! everything scoped to a realm. Where cave-auth's surface diverges
//! from upstream (e.g. groups + idp + flows are stubbed in cave-auth
//! 2026-05-15 but Portal still surfaces the tabs), the mock simply
//! holds the shape and the live client surfaces `ClientError::NotWired`.
//!
//! Source: keycloak/keycloak@v22.0.0
//!         services/src/main/java/org/keycloak/services/resources/admin/AdminRoot.java

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;

// ── error type ────────────────────────────────────────────────────────────────

/// Errors returned by `AuthClient` implementations.
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

impl ClientError {
    /// Map an HTTP status to the user-visible classification used by
    /// the optimistic-update path. 409 → Conflict (Retry), 404 → NotFound,
    /// everything else → generic Status.
    pub fn from_status(status: reqwest::StatusCode, body: String) -> Self {
        if status == reqwest::StatusCode::CONFLICT {
            Self::Conflict(body)
        } else if status == reqwest::StatusCode::NOT_FOUND {
            Self::NotFound(body)
        } else {
            Self::Status { status, body }
        }
    }
}

// ── DTOs ──────────────────────────────────────────────────────────────────────
// Shape-compatible with Keycloak admin REST. Only the fields Portal renders
// are modelled; everything else is dropped via `#[serde(default)]`.

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
    #[serde(default, rename = "publicClient")]
    pub public_client: bool,
    #[serde(default, rename = "redirectUris")]
    pub redirect_uris: Vec<String>,
    #[serde(default)]
    pub protocol: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct User {
    pub id: String,
    #[serde(default)]
    pub realm: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default, rename = "emailVerified")]
    pub email_verified: bool,
    #[serde(default, rename = "firstName")]
    pub first_name: Option<String>,
    #[serde(default, rename = "lastName")]
    pub last_name: Option<String>,
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Role {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub realm: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub composite: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Group {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub realm: String,
    #[serde(default)]
    pub path: String,
    #[serde(default, rename = "subGroupCount")]
    pub sub_group_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct IdentityProvider {
    pub alias: String,
    #[serde(default)]
    pub realm: String,
    #[serde(default, rename = "displayName")]
    pub display_name: String,
    /// e.g. "saml" | "oidc" | "github" | "google" | "ldap" | "kerberos".
    #[serde(default, rename = "providerId")]
    pub provider_id: String,
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AuthFlow {
    pub id: String,
    pub alias: String,
    #[serde(default)]
    pub realm: String,
    #[serde(default)]
    pub description: Option<String>,
    /// e.g. "basic-flow" | "client-flow"
    #[serde(default, rename = "providerId")]
    pub provider_id: String,
    /// `true` = bundled Keycloak flow that operators cannot edit.
    #[serde(default, rename = "builtIn")]
    pub built_in: bool,
}

/// Admin event payload. Matches Keycloak's `AdminEventRepresentation`.
///
/// Source: keycloak/keycloak@v22.0.0
///         services/src/main/java/org/keycloak/events/admin/AdminEvent.java
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct EventPayload {
    pub time: i64,
    pub realm: String,
    /// e.g. "LOGIN" | "LOGIN_ERROR" | "LOGOUT" | "CREATE_USER" | ...
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, rename = "operationType")]
    pub operation_type: Option<String>,
    #[serde(default, rename = "resourceType")]
    pub resource_type: Option<String>,
    #[serde(default, rename = "resourcePath")]
    pub resource_path: Option<String>,
    #[serde(default, rename = "userId")]
    pub user_id: Option<String>,
    #[serde(default, rename = "ipAddress")]
    pub ip_address: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct UserSession {
    pub id: String,
    #[serde(default)]
    pub realm: String,
    #[serde(default, rename = "userId")]
    pub user_id: String,
    #[serde(default)]
    pub username: String,
    #[serde(default, rename = "ipAddress")]
    pub ip_address: String,
    #[serde(default)]
    pub start: i64,
    #[serde(default, rename = "lastAccess")]
    pub last_access: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AccountProfile {
    pub username: String,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default, rename = "firstName")]
    pub first_name: Option<String>,
    #[serde(default, rename = "lastName")]
    pub last_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Credential {
    pub id: String,
    /// "password" | "otp" | "webauthn-passwordless" | "webauthn-platform"
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, rename = "userLabel")]
    pub user_label: Option<String>,
    #[serde(default, rename = "createdDate")]
    pub created_date: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct LinkedApplication {
    #[serde(rename = "clientId")]
    pub client_id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default, rename = "inUse")]
    pub in_use: bool,
    #[serde(default, rename = "consentRequired")]
    pub consent_required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct FederatedIdentity {
    #[serde(rename = "identityProvider")]
    pub identity_provider: String,
    #[serde(default, rename = "userId")]
    pub user_id: String,
    #[serde(default, rename = "userName")]
    pub user_name: String,
}

// ── trait ────────────────────────────────────────────────────────────────────

/// Live data source for cave-auth admin REST endpoints. All methods
/// are realm-scoped (except realm CRUD itself) and async.
#[async_trait]
pub trait AuthClient: Send + Sync + std::fmt::Debug {
    // realms — /admin/realms
    async fn list_realms(&self) -> Result<Vec<Realm>, ClientError>;
    async fn get_realm(&self, name: &str) -> Result<Realm, ClientError>;
    async fn create_realm(&self, realm: &Realm) -> Result<Realm, ClientError>;
    async fn update_realm(&self, realm: &Realm) -> Result<Realm, ClientError>;
    async fn delete_realm(&self, name: &str) -> Result<(), ClientError>;

    // clients — /admin/realms/{realm}/clients
    async fn list_clients(&self, realm: &str) -> Result<Vec<ClientApp>, ClientError>;
    async fn get_client(&self, realm: &str, id: &str) -> Result<ClientApp, ClientError>;
    async fn create_client(
        &self,
        realm: &str,
        client: &ClientApp,
    ) -> Result<ClientApp, ClientError>;
    async fn update_client(
        &self,
        realm: &str,
        client: &ClientApp,
    ) -> Result<ClientApp, ClientError>;
    async fn delete_client(&self, realm: &str, id: &str) -> Result<(), ClientError>;

    // users — /admin/realms/{realm}/users
    async fn list_users(&self, realm: &str) -> Result<Vec<User>, ClientError>;
    async fn get_user(&self, realm: &str, id: &str) -> Result<User, ClientError>;
    async fn create_user(&self, realm: &str, user: &User) -> Result<User, ClientError>;
    async fn update_user(&self, realm: &str, user: &User) -> Result<User, ClientError>;
    async fn delete_user(&self, realm: &str, id: &str) -> Result<(), ClientError>;

    // roles — /admin/realms/{realm}/roles
    async fn list_roles(&self, realm: &str) -> Result<Vec<Role>, ClientError>;
    async fn get_role(&self, realm: &str, name: &str) -> Result<Role, ClientError>;
    async fn create_role(&self, realm: &str, role: &Role) -> Result<Role, ClientError>;
    async fn update_role(&self, realm: &str, role: &Role) -> Result<Role, ClientError>;
    async fn delete_role(&self, realm: &str, name: &str) -> Result<(), ClientError>;

    // groups — /admin/realms/{realm}/groups
    async fn list_groups(&self, realm: &str) -> Result<Vec<Group>, ClientError>;
    async fn get_group(&self, realm: &str, id: &str) -> Result<Group, ClientError>;
    async fn create_group(&self, realm: &str, group: &Group) -> Result<Group, ClientError>;
    async fn update_group(&self, realm: &str, group: &Group) -> Result<Group, ClientError>;
    async fn delete_group(&self, realm: &str, id: &str) -> Result<(), ClientError>;

    // identity providers — /admin/realms/{realm}/identity-provider/instances
    async fn list_identity_providers(
        &self,
        realm: &str,
    ) -> Result<Vec<IdentityProvider>, ClientError>;
    async fn get_identity_provider(
        &self,
        realm: &str,
        alias: &str,
    ) -> Result<IdentityProvider, ClientError>;
    async fn create_identity_provider(
        &self,
        realm: &str,
        idp: &IdentityProvider,
    ) -> Result<IdentityProvider, ClientError>;
    async fn update_identity_provider(
        &self,
        realm: &str,
        idp: &IdentityProvider,
    ) -> Result<IdentityProvider, ClientError>;
    async fn delete_identity_provider(
        &self,
        realm: &str,
        alias: &str,
    ) -> Result<(), ClientError>;

    // auth flows — /admin/realms/{realm}/authentication/flows
    async fn list_auth_flows(&self, realm: &str) -> Result<Vec<AuthFlow>, ClientError>;
    async fn get_auth_flow(&self, realm: &str, id: &str) -> Result<AuthFlow, ClientError>;
    async fn create_auth_flow(&self, realm: &str, flow: &AuthFlow) -> Result<AuthFlow, ClientError>;
    async fn update_auth_flow(&self, realm: &str, flow: &AuthFlow) -> Result<AuthFlow, ClientError>;
    async fn delete_auth_flow(&self, realm: &str, id: &str) -> Result<(), ClientError>;

    // events — /admin/realms/{realm}/events
    async fn list_events(
        &self,
        realm: &str,
        since: Option<i64>,
        kinds: &[&str],
    ) -> Result<Vec<EventPayload>, ClientError>;

    // sessions — /admin/realms/{realm}/sessions
    async fn list_sessions(&self, realm: &str) -> Result<Vec<UserSession>, ClientError>;
    async fn delete_session(&self, realm: &str, id: &str) -> Result<(), ClientError>;

    // account console — /realms/{realm}/account/*
    async fn get_account_profile(
        &self,
        realm: &str,
        username: &str,
    ) -> Result<AccountProfile, ClientError>;
    async fn update_account_profile(
        &self,
        realm: &str,
        username: &str,
        profile: &AccountProfile,
    ) -> Result<AccountProfile, ClientError>;
    async fn list_account_sessions(
        &self,
        realm: &str,
        username: &str,
    ) -> Result<Vec<UserSession>, ClientError>;
    async fn list_account_credentials(
        &self,
        realm: &str,
        username: &str,
    ) -> Result<Vec<Credential>, ClientError>;
    async fn list_account_applications(
        &self,
        realm: &str,
        username: &str,
    ) -> Result<Vec<LinkedApplication>, ClientError>;
    async fn list_account_federated_identities(
        &self,
        realm: &str,
        username: &str,
    ) -> Result<Vec<FederatedIdentity>, ClientError>;

    /// Subscribe to a Server-Sent-Events stream of admin events on the
    /// given realm. Returns a `tokio::sync::mpsc::Receiver` so the
    /// caller can `await` events without blocking the runtime.
    ///
    /// The implementation may spawn a long-running task. Drop the
    /// receiver to cancel the subscription.
    async fn subscribe_events(
        &self,
        realm: &str,
    ) -> Result<tokio::sync::mpsc::Receiver<EventPayload>, ClientError>;
}

/// Shared alias.
pub type SharedAuthClient = Arc<dyn AuthClient>;

// ── AuthMockClient ────────────────────────────────────────────────────────────

/// In-memory backing store for tests and the dev `?tenant_id=...` flow.
///
/// Thread-safe via interior `RwLock`s. Every CRUD path mirrors the live
/// client's error surface (`NotFound`, `Conflict`) so handler tests can
/// exercise both happy and unhappy paths without needing an httpmock.
#[derive(Debug)]
pub struct AuthMockClient {
    realms: RwLock<HashMap<String, Realm>>,
    clients: RwLock<HashMap<(String, String), ClientApp>>,
    users: RwLock<HashMap<(String, String), User>>,
    roles: RwLock<HashMap<(String, String), Role>>,
    groups: RwLock<HashMap<(String, String), Group>>,
    idps: RwLock<HashMap<(String, String), IdentityProvider>>,
    flows: RwLock<HashMap<(String, String), AuthFlow>>,
    sessions: RwLock<HashMap<(String, String), UserSession>>,
    events: RwLock<Vec<EventPayload>>,
    profiles: RwLock<HashMap<(String, String), AccountProfile>>,
    credentials: RwLock<HashMap<(String, String), Vec<Credential>>>,
    apps: RwLock<HashMap<(String, String), Vec<LinkedApplication>>>,
    federated: RwLock<HashMap<(String, String), Vec<FederatedIdentity>>>,
    event_bus: tokio::sync::broadcast::Sender<EventPayload>,
}

impl Default for AuthMockClient {
    fn default() -> Self {
        Self::new()
    }
}

impl AuthMockClient {
    pub fn new() -> Self {
        let (tx, _) = tokio::sync::broadcast::channel(64);
        Self {
            realms: RwLock::new(HashMap::new()),
            clients: RwLock::new(HashMap::new()),
            users: RwLock::new(HashMap::new()),
            roles: RwLock::new(HashMap::new()),
            groups: RwLock::new(HashMap::new()),
            idps: RwLock::new(HashMap::new()),
            flows: RwLock::new(HashMap::new()),
            sessions: RwLock::new(HashMap::new()),
            events: RwLock::new(Vec::new()),
            profiles: RwLock::new(HashMap::new()),
            credentials: RwLock::new(HashMap::new()),
            apps: RwLock::new(HashMap::new()),
            federated: RwLock::new(HashMap::new()),
            event_bus: tx,
        }
    }

    /// Helper used by tests + setup wiring — seed a single realm.
    pub fn seed_realm(&self, realm: Realm) {
        self.realms.write().unwrap().insert(realm.id.clone(), realm);
    }

    /// Push an event into the bus so subscribers see it. Also appended
    /// to the events log so `list_events` returns it.
    pub fn push_event(&self, evt: EventPayload) {
        self.events.write().unwrap().push(evt.clone());
        let _ = self.event_bus.send(evt);
    }

    pub fn seed_session(&self, sess: UserSession) {
        self.sessions
            .write()
            .unwrap()
            .insert((sess.realm.clone(), sess.id.clone()), sess);
    }

    pub fn seed_user(&self, user: User) {
        self.users
            .write()
            .unwrap()
            .insert((user.realm.clone(), user.id.clone()), user);
    }

    pub fn seed_role(&self, role: Role) {
        self.roles
            .write()
            .unwrap()
            .insert((role.realm.clone(), role.name.clone()), role);
    }

    pub fn seed_group(&self, group: Group) {
        self.groups
            .write()
            .unwrap()
            .insert((group.realm.clone(), group.id.clone()), group);
    }

    pub fn seed_idp(&self, idp: IdentityProvider) {
        self.idps
            .write()
            .unwrap()
            .insert((idp.realm.clone(), idp.alias.clone()), idp);
    }

    pub fn seed_flow(&self, flow: AuthFlow) {
        self.flows
            .write()
            .unwrap()
            .insert((flow.realm.clone(), flow.id.clone()), flow);
    }

    pub fn seed_credential(&self, realm: &str, username: &str, cred: Credential) {
        self.credentials
            .write()
            .unwrap()
            .entry((realm.to_string(), username.to_string()))
            .or_default()
            .push(cred);
    }

    pub fn seed_application(&self, realm: &str, username: &str, app: LinkedApplication) {
        self.apps
            .write()
            .unwrap()
            .entry((realm.to_string(), username.to_string()))
            .or_default()
            .push(app);
    }

    pub fn seed_profile(&self, realm: &str, profile: AccountProfile) {
        self.profiles
            .write()
            .unwrap()
            .insert((realm.to_string(), profile.username.clone()), profile);
    }
}

#[async_trait]
impl AuthClient for AuthMockClient {
    async fn list_realms(&self) -> Result<Vec<Realm>, ClientError> {
        let mut v: Vec<_> = self.realms.read().unwrap().values().cloned().collect();
        v.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(v)
    }
    async fn get_realm(&self, name: &str) -> Result<Realm, ClientError> {
        self.realms
            .read()
            .unwrap()
            .get(name)
            .cloned()
            .ok_or_else(|| ClientError::NotFound(format!("realm/{name}")))
    }
    async fn create_realm(&self, realm: &Realm) -> Result<Realm, ClientError> {
        let mut w = self.realms.write().unwrap();
        if w.contains_key(&realm.id) {
            return Err(ClientError::Conflict(format!("realm/{}", realm.id)));
        }
        w.insert(realm.id.clone(), realm.clone());
        Ok(realm.clone())
    }
    async fn update_realm(&self, realm: &Realm) -> Result<Realm, ClientError> {
        let mut w = self.realms.write().unwrap();
        if !w.contains_key(&realm.id) {
            return Err(ClientError::NotFound(format!("realm/{}", realm.id)));
        }
        w.insert(realm.id.clone(), realm.clone());
        Ok(realm.clone())
    }
    async fn delete_realm(&self, name: &str) -> Result<(), ClientError> {
        self.realms
            .write()
            .unwrap()
            .remove(name)
            .map(|_| ())
            .ok_or_else(|| ClientError::NotFound(format!("realm/{name}")))
    }

    async fn list_clients(&self, realm: &str) -> Result<Vec<ClientApp>, ClientError> {
        let mut v: Vec<_> = self
            .clients
            .read()
            .unwrap()
            .values()
            .filter(|c| c.realm == realm)
            .cloned()
            .collect();
        v.sort_by(|a, b| a.client_id.cmp(&b.client_id));
        Ok(v)
    }
    async fn get_client(&self, realm: &str, id: &str) -> Result<ClientApp, ClientError> {
        self.clients
            .read()
            .unwrap()
            .get(&(realm.to_string(), id.to_string()))
            .cloned()
            .ok_or_else(|| ClientError::NotFound(format!("client/{realm}/{id}")))
    }
    async fn create_client(
        &self,
        realm: &str,
        client: &ClientApp,
    ) -> Result<ClientApp, ClientError> {
        let mut w = self.clients.write().unwrap();
        let key = (realm.to_string(), client.id.clone());
        if w.contains_key(&key) {
            return Err(ClientError::Conflict(format!("client/{realm}/{}", client.id)));
        }
        let mut stored = client.clone();
        stored.realm = realm.to_string();
        w.insert(key, stored.clone());
        Ok(stored)
    }
    async fn update_client(
        &self,
        realm: &str,
        client: &ClientApp,
    ) -> Result<ClientApp, ClientError> {
        let mut w = self.clients.write().unwrap();
        let key = (realm.to_string(), client.id.clone());
        if !w.contains_key(&key) {
            return Err(ClientError::NotFound(format!("client/{realm}/{}", client.id)));
        }
        let mut stored = client.clone();
        stored.realm = realm.to_string();
        w.insert(key, stored.clone());
        Ok(stored)
    }
    async fn delete_client(&self, realm: &str, id: &str) -> Result<(), ClientError> {
        self.clients
            .write()
            .unwrap()
            .remove(&(realm.to_string(), id.to_string()))
            .map(|_| ())
            .ok_or_else(|| ClientError::NotFound(format!("client/{realm}/{id}")))
    }

    async fn list_users(&self, realm: &str) -> Result<Vec<User>, ClientError> {
        let mut v: Vec<_> = self
            .users
            .read()
            .unwrap()
            .values()
            .filter(|u| u.realm == realm)
            .cloned()
            .collect();
        v.sort_by(|a, b| a.username.cmp(&b.username));
        Ok(v)
    }
    async fn get_user(&self, realm: &str, id: &str) -> Result<User, ClientError> {
        self.users
            .read()
            .unwrap()
            .get(&(realm.to_string(), id.to_string()))
            .cloned()
            .ok_or_else(|| ClientError::NotFound(format!("user/{realm}/{id}")))
    }
    async fn create_user(&self, realm: &str, user: &User) -> Result<User, ClientError> {
        let mut w = self.users.write().unwrap();
        let key = (realm.to_string(), user.id.clone());
        if w.contains_key(&key) {
            return Err(ClientError::Conflict(format!("user/{realm}/{}", user.id)));
        }
        let mut stored = user.clone();
        stored.realm = realm.to_string();
        w.insert(key, stored.clone());
        Ok(stored)
    }
    async fn update_user(&self, realm: &str, user: &User) -> Result<User, ClientError> {
        let mut w = self.users.write().unwrap();
        let key = (realm.to_string(), user.id.clone());
        if !w.contains_key(&key) {
            return Err(ClientError::NotFound(format!("user/{realm}/{}", user.id)));
        }
        let mut stored = user.clone();
        stored.realm = realm.to_string();
        w.insert(key, stored.clone());
        Ok(stored)
    }
    async fn delete_user(&self, realm: &str, id: &str) -> Result<(), ClientError> {
        self.users
            .write()
            .unwrap()
            .remove(&(realm.to_string(), id.to_string()))
            .map(|_| ())
            .ok_or_else(|| ClientError::NotFound(format!("user/{realm}/{id}")))
    }

    async fn list_roles(&self, realm: &str) -> Result<Vec<Role>, ClientError> {
        let mut v: Vec<_> = self
            .roles
            .read()
            .unwrap()
            .values()
            .filter(|r| r.realm == realm)
            .cloned()
            .collect();
        v.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(v)
    }
    async fn get_role(&self, realm: &str, name: &str) -> Result<Role, ClientError> {
        self.roles
            .read()
            .unwrap()
            .get(&(realm.to_string(), name.to_string()))
            .cloned()
            .ok_or_else(|| ClientError::NotFound(format!("role/{realm}/{name}")))
    }
    async fn create_role(&self, realm: &str, role: &Role) -> Result<Role, ClientError> {
        let mut w = self.roles.write().unwrap();
        let key = (realm.to_string(), role.name.clone());
        if w.contains_key(&key) {
            return Err(ClientError::Conflict(format!("role/{realm}/{}", role.name)));
        }
        let mut stored = role.clone();
        stored.realm = realm.to_string();
        w.insert(key, stored.clone());
        Ok(stored)
    }
    async fn update_role(&self, realm: &str, role: &Role) -> Result<Role, ClientError> {
        let mut w = self.roles.write().unwrap();
        let key = (realm.to_string(), role.name.clone());
        if !w.contains_key(&key) {
            return Err(ClientError::NotFound(format!("role/{realm}/{}", role.name)));
        }
        let mut stored = role.clone();
        stored.realm = realm.to_string();
        w.insert(key, stored.clone());
        Ok(stored)
    }
    async fn delete_role(&self, realm: &str, name: &str) -> Result<(), ClientError> {
        self.roles
            .write()
            .unwrap()
            .remove(&(realm.to_string(), name.to_string()))
            .map(|_| ())
            .ok_or_else(|| ClientError::NotFound(format!("role/{realm}/{name}")))
    }

    async fn list_groups(&self, realm: &str) -> Result<Vec<Group>, ClientError> {
        let mut v: Vec<_> = self
            .groups
            .read()
            .unwrap()
            .values()
            .filter(|g| g.realm == realm)
            .cloned()
            .collect();
        v.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(v)
    }
    async fn get_group(&self, realm: &str, id: &str) -> Result<Group, ClientError> {
        self.groups
            .read()
            .unwrap()
            .get(&(realm.to_string(), id.to_string()))
            .cloned()
            .ok_or_else(|| ClientError::NotFound(format!("group/{realm}/{id}")))
    }
    async fn create_group(&self, realm: &str, group: &Group) -> Result<Group, ClientError> {
        let mut w = self.groups.write().unwrap();
        let key = (realm.to_string(), group.id.clone());
        if w.contains_key(&key) {
            return Err(ClientError::Conflict(format!("group/{realm}/{}", group.id)));
        }
        let mut stored = group.clone();
        stored.realm = realm.to_string();
        w.insert(key, stored.clone());
        Ok(stored)
    }
    async fn update_group(&self, realm: &str, group: &Group) -> Result<Group, ClientError> {
        let mut w = self.groups.write().unwrap();
        let key = (realm.to_string(), group.id.clone());
        if !w.contains_key(&key) {
            return Err(ClientError::NotFound(format!("group/{realm}/{}", group.id)));
        }
        let mut stored = group.clone();
        stored.realm = realm.to_string();
        w.insert(key, stored.clone());
        Ok(stored)
    }
    async fn delete_group(&self, realm: &str, id: &str) -> Result<(), ClientError> {
        self.groups
            .write()
            .unwrap()
            .remove(&(realm.to_string(), id.to_string()))
            .map(|_| ())
            .ok_or_else(|| ClientError::NotFound(format!("group/{realm}/{id}")))
    }

    async fn list_identity_providers(
        &self,
        realm: &str,
    ) -> Result<Vec<IdentityProvider>, ClientError> {
        let mut v: Vec<_> = self
            .idps
            .read()
            .unwrap()
            .values()
            .filter(|i| i.realm == realm)
            .cloned()
            .collect();
        v.sort_by(|a, b| a.alias.cmp(&b.alias));
        Ok(v)
    }
    async fn get_identity_provider(
        &self,
        realm: &str,
        alias: &str,
    ) -> Result<IdentityProvider, ClientError> {
        self.idps
            .read()
            .unwrap()
            .get(&(realm.to_string(), alias.to_string()))
            .cloned()
            .ok_or_else(|| ClientError::NotFound(format!("idp/{realm}/{alias}")))
    }
    async fn create_identity_provider(
        &self,
        realm: &str,
        idp: &IdentityProvider,
    ) -> Result<IdentityProvider, ClientError> {
        let mut w = self.idps.write().unwrap();
        let key = (realm.to_string(), idp.alias.clone());
        if w.contains_key(&key) {
            return Err(ClientError::Conflict(format!("idp/{realm}/{}", idp.alias)));
        }
        let mut stored = idp.clone();
        stored.realm = realm.to_string();
        w.insert(key, stored.clone());
        Ok(stored)
    }
    async fn update_identity_provider(
        &self,
        realm: &str,
        idp: &IdentityProvider,
    ) -> Result<IdentityProvider, ClientError> {
        let mut w = self.idps.write().unwrap();
        let key = (realm.to_string(), idp.alias.clone());
        if !w.contains_key(&key) {
            return Err(ClientError::NotFound(format!("idp/{realm}/{}", idp.alias)));
        }
        let mut stored = idp.clone();
        stored.realm = realm.to_string();
        w.insert(key, stored.clone());
        Ok(stored)
    }
    async fn delete_identity_provider(
        &self,
        realm: &str,
        alias: &str,
    ) -> Result<(), ClientError> {
        self.idps
            .write()
            .unwrap()
            .remove(&(realm.to_string(), alias.to_string()))
            .map(|_| ())
            .ok_or_else(|| ClientError::NotFound(format!("idp/{realm}/{alias}")))
    }

    async fn list_auth_flows(&self, realm: &str) -> Result<Vec<AuthFlow>, ClientError> {
        let mut v: Vec<_> = self
            .flows
            .read()
            .unwrap()
            .values()
            .filter(|f| f.realm == realm)
            .cloned()
            .collect();
        v.sort_by(|a, b| a.alias.cmp(&b.alias));
        Ok(v)
    }
    async fn get_auth_flow(&self, realm: &str, id: &str) -> Result<AuthFlow, ClientError> {
        self.flows
            .read()
            .unwrap()
            .get(&(realm.to_string(), id.to_string()))
            .cloned()
            .ok_or_else(|| ClientError::NotFound(format!("flow/{realm}/{id}")))
    }
    async fn create_auth_flow(
        &self,
        realm: &str,
        flow: &AuthFlow,
    ) -> Result<AuthFlow, ClientError> {
        let mut w = self.flows.write().unwrap();
        let key = (realm.to_string(), flow.id.clone());
        if w.contains_key(&key) {
            return Err(ClientError::Conflict(format!("flow/{realm}/{}", flow.id)));
        }
        let mut stored = flow.clone();
        stored.realm = realm.to_string();
        w.insert(key, stored.clone());
        Ok(stored)
    }
    async fn update_auth_flow(
        &self,
        realm: &str,
        flow: &AuthFlow,
    ) -> Result<AuthFlow, ClientError> {
        let mut w = self.flows.write().unwrap();
        let key = (realm.to_string(), flow.id.clone());
        if !w.contains_key(&key) {
            return Err(ClientError::NotFound(format!("flow/{realm}/{}", flow.id)));
        }
        let mut stored = flow.clone();
        stored.realm = realm.to_string();
        w.insert(key, stored.clone());
        Ok(stored)
    }
    async fn delete_auth_flow(&self, realm: &str, id: &str) -> Result<(), ClientError> {
        self.flows
            .write()
            .unwrap()
            .remove(&(realm.to_string(), id.to_string()))
            .map(|_| ())
            .ok_or_else(|| ClientError::NotFound(format!("flow/{realm}/{id}")))
    }

    async fn list_events(
        &self,
        realm: &str,
        since: Option<i64>,
        kinds: &[&str],
    ) -> Result<Vec<EventPayload>, ClientError> {
        let mut v: Vec<_> = self
            .events
            .read()
            .unwrap()
            .iter()
            .filter(|e| e.realm == realm)
            .filter(|e| since.map(|t| e.time >= t).unwrap_or(true))
            .filter(|e| kinds.is_empty() || kinds.iter().any(|k| *k == e.kind))
            .cloned()
            .collect();
        v.sort_by(|a, b| a.time.cmp(&b.time));
        Ok(v)
    }

    async fn list_sessions(&self, realm: &str) -> Result<Vec<UserSession>, ClientError> {
        let mut v: Vec<_> = self
            .sessions
            .read()
            .unwrap()
            .values()
            .filter(|s| s.realm == realm)
            .cloned()
            .collect();
        v.sort_by(|a, b| a.start.cmp(&b.start));
        Ok(v)
    }
    async fn delete_session(&self, realm: &str, id: &str) -> Result<(), ClientError> {
        self.sessions
            .write()
            .unwrap()
            .remove(&(realm.to_string(), id.to_string()))
            .map(|_| ())
            .ok_or_else(|| ClientError::NotFound(format!("session/{realm}/{id}")))
    }

    async fn get_account_profile(
        &self,
        realm: &str,
        username: &str,
    ) -> Result<AccountProfile, ClientError> {
        self.profiles
            .read()
            .unwrap()
            .get(&(realm.to_string(), username.to_string()))
            .cloned()
            .ok_or_else(|| ClientError::NotFound(format!("account/{realm}/{username}")))
    }
    async fn update_account_profile(
        &self,
        realm: &str,
        username: &str,
        profile: &AccountProfile,
    ) -> Result<AccountProfile, ClientError> {
        let mut w = self.profiles.write().unwrap();
        let key = (realm.to_string(), username.to_string());
        if !w.contains_key(&key) {
            return Err(ClientError::NotFound(format!("account/{realm}/{username}")));
        }
        let mut stored = profile.clone();
        stored.username = username.to_string();
        w.insert(key, stored.clone());
        Ok(stored)
    }
    async fn list_account_sessions(
        &self,
        realm: &str,
        username: &str,
    ) -> Result<Vec<UserSession>, ClientError> {
        let mut v: Vec<_> = self
            .sessions
            .read()
            .unwrap()
            .values()
            .filter(|s| s.realm == realm && s.username == username)
            .cloned()
            .collect();
        v.sort_by(|a, b| a.start.cmp(&b.start));
        Ok(v)
    }
    async fn list_account_credentials(
        &self,
        realm: &str,
        username: &str,
    ) -> Result<Vec<Credential>, ClientError> {
        Ok(self
            .credentials
            .read()
            .unwrap()
            .get(&(realm.to_string(), username.to_string()))
            .cloned()
            .unwrap_or_default())
    }
    async fn list_account_applications(
        &self,
        realm: &str,
        username: &str,
    ) -> Result<Vec<LinkedApplication>, ClientError> {
        Ok(self
            .apps
            .read()
            .unwrap()
            .get(&(realm.to_string(), username.to_string()))
            .cloned()
            .unwrap_or_default())
    }
    async fn list_account_federated_identities(
        &self,
        realm: &str,
        username: &str,
    ) -> Result<Vec<FederatedIdentity>, ClientError> {
        Ok(self
            .federated
            .read()
            .unwrap()
            .get(&(realm.to_string(), username.to_string()))
            .cloned()
            .unwrap_or_default())
    }

    async fn subscribe_events(
        &self,
        realm: &str,
    ) -> Result<tokio::sync::mpsc::Receiver<EventPayload>, ClientError> {
        let (tx, rx) = tokio::sync::mpsc::channel(64);
        let mut bus_rx = self.event_bus.subscribe();
        let realm = realm.to_string();
        tokio::spawn(async move {
            while let Ok(evt) = bus_rx.recv().await {
                if evt.realm == realm && tx.send(evt).await.is_err() {
                    break;
                }
            }
        });
        Ok(rx)
    }
}

// ── AuthApiClient (reqwest) ───────────────────────────────────────────────────

/// Live cave-auth admin REST client. CA-pinned mTLS via the same
/// helper shape as [`crate::admin::runtime_client::ApiserverClient`].
///
/// Source: keycloak/keycloak@v22.0.0
///         services/src/main/java/org/keycloak/services/resources/admin/AdminRoot.java
#[derive(Debug)]
pub struct AuthApiClient {
    client: reqwest::Client,
    base_url: String,
    bearer_token: Option<String>,
}

impl AuthApiClient {
    /// Build from a CA bundle (PEM) + optional client identity (PEM).
    pub fn new(
        base_url: String,
        ca_pem: &[u8],
        client_identity_pem: Option<&[u8]>,
        bearer_token: Option<String>,
        request_timeout: Duration,
    ) -> Result<Self, ClientError> {
        let ca = reqwest::Certificate::from_pem(ca_pem)
            .map_err(|e| ClientError::Decode(format!("CA: {e}")))?;
        let mut builder = reqwest::Client::builder()
            .add_root_certificate(ca)
            .timeout(request_timeout)
            .https_only(true)
            .tls_built_in_root_certs(false);
        if let Some(identity) = client_identity_pem {
            let id = reqwest::Identity::from_pem(identity)
                .map_err(|e| ClientError::Decode(format!("identity: {e}")))?;
            builder = builder.identity(id);
        }
        let client = builder
            .build()
            .map_err(|e| ClientError::Decode(format!("build: {e}")))?;
        Ok(Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            bearer_token,
        })
    }

    /// Test-only constructor — accepts invalid certs so `httpmock`
    /// servers work without TLS plumbing. The shape matches
    /// `ApiserverClient::test_against`.
    #[cfg(test)]
    pub fn test_against(base_url: String) -> Self {
        let client = reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .timeout(Duration::from_secs(2))
            .build()
            .expect("test client");
        Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            bearer_token: None,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    fn auth(&self, mut req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if let Some(t) = &self.bearer_token {
            req = req.bearer_auth(t);
        }
        req
    }

    async fn handle_resp<T: serde::de::DeserializeOwned>(
        resp: reqwest::Response,
    ) -> Result<T, ClientError> {
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(ClientError::from_status(status, body));
        }
        let bytes = resp.bytes().await?;
        serde_json::from_slice::<T>(&bytes).map_err(|e| ClientError::Decode(e.to_string()))
    }

    async fn handle_empty(resp: reqwest::Response) -> Result<(), ClientError> {
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(ClientError::from_status(status, body));
        }
        Ok(())
    }

    async fn get_json<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, ClientError> {
        let resp = self.auth(self.client.get(self.url(path))).send().await?;
        Self::handle_resp(resp).await
    }

    async fn post_json<I: Serialize, O: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: &I,
    ) -> Result<O, ClientError> {
        let resp = self
            .auth(self.client.post(self.url(path)))
            .json(body)
            .send()
            .await?;
        Self::handle_resp(resp).await
    }

    async fn put_json<I: Serialize, O: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: &I,
    ) -> Result<O, ClientError> {
        let resp = self
            .auth(self.client.put(self.url(path)))
            .json(body)
            .send()
            .await?;
        Self::handle_resp(resp).await
    }

    async fn delete_path(&self, path: &str) -> Result<(), ClientError> {
        let resp = self.auth(self.client.delete(self.url(path))).send().await?;
        Self::handle_empty(resp).await
    }
}

#[async_trait]
impl AuthClient for AuthApiClient {
    // realms

    /// Source: keycloak/keycloak@v22.0.0
    ///         services/src/main/java/org/keycloak/services/resources/admin/RealmsAdminResource.java
    async fn list_realms(&self) -> Result<Vec<Realm>, ClientError> {
        self.get_json("/admin/realms").await
    }
    async fn get_realm(&self, name: &str) -> Result<Realm, ClientError> {
        self.get_json(&format!("/admin/realms/{name}")).await
    }
    async fn create_realm(&self, realm: &Realm) -> Result<Realm, ClientError> {
        self.post_json("/admin/realms", realm).await
    }
    async fn update_realm(&self, realm: &Realm) -> Result<Realm, ClientError> {
        self.put_json(&format!("/admin/realms/{}", realm.id), realm).await
    }
    async fn delete_realm(&self, name: &str) -> Result<(), ClientError> {
        self.delete_path(&format!("/admin/realms/{name}")).await
    }

    // clients
    /// Source: keycloak/keycloak@v22.0.0
    ///         services/src/main/java/org/keycloak/services/resources/admin/ClientsResource.java
    async fn list_clients(&self, realm: &str) -> Result<Vec<ClientApp>, ClientError> {
        self.get_json(&format!("/admin/realms/{realm}/clients")).await
    }
    async fn get_client(&self, realm: &str, id: &str) -> Result<ClientApp, ClientError> {
        self.get_json(&format!("/admin/realms/{realm}/clients/{id}")).await
    }
    async fn create_client(
        &self,
        realm: &str,
        client: &ClientApp,
    ) -> Result<ClientApp, ClientError> {
        self.post_json(&format!("/admin/realms/{realm}/clients"), client).await
    }
    async fn update_client(
        &self,
        realm: &str,
        client: &ClientApp,
    ) -> Result<ClientApp, ClientError> {
        self.put_json(&format!("/admin/realms/{realm}/clients/{}", client.id), client)
            .await
    }
    async fn delete_client(&self, realm: &str, id: &str) -> Result<(), ClientError> {
        self.delete_path(&format!("/admin/realms/{realm}/clients/{id}")).await
    }

    // users
    /// Source: keycloak/keycloak@v22.0.0
    ///         services/src/main/java/org/keycloak/services/resources/admin/UsersResource.java
    async fn list_users(&self, realm: &str) -> Result<Vec<User>, ClientError> {
        self.get_json(&format!("/admin/realms/{realm}/users")).await
    }
    async fn get_user(&self, realm: &str, id: &str) -> Result<User, ClientError> {
        self.get_json(&format!("/admin/realms/{realm}/users/{id}")).await
    }
    async fn create_user(&self, realm: &str, user: &User) -> Result<User, ClientError> {
        self.post_json(&format!("/admin/realms/{realm}/users"), user).await
    }
    async fn update_user(&self, realm: &str, user: &User) -> Result<User, ClientError> {
        self.put_json(&format!("/admin/realms/{realm}/users/{}", user.id), user).await
    }
    async fn delete_user(&self, realm: &str, id: &str) -> Result<(), ClientError> {
        self.delete_path(&format!("/admin/realms/{realm}/users/{id}")).await
    }

    // roles
    /// Source: keycloak/keycloak@v22.0.0
    ///         services/src/main/java/org/keycloak/services/resources/admin/RoleContainerResource.java
    async fn list_roles(&self, realm: &str) -> Result<Vec<Role>, ClientError> {
        self.get_json(&format!("/admin/realms/{realm}/roles")).await
    }
    async fn get_role(&self, realm: &str, name: &str) -> Result<Role, ClientError> {
        self.get_json(&format!("/admin/realms/{realm}/roles/{name}")).await
    }
    async fn create_role(&self, realm: &str, role: &Role) -> Result<Role, ClientError> {
        self.post_json(&format!("/admin/realms/{realm}/roles"), role).await
    }
    async fn update_role(&self, realm: &str, role: &Role) -> Result<Role, ClientError> {
        self.put_json(&format!("/admin/realms/{realm}/roles/{}", role.name), role)
            .await
    }
    async fn delete_role(&self, realm: &str, name: &str) -> Result<(), ClientError> {
        self.delete_path(&format!("/admin/realms/{realm}/roles/{name}")).await
    }

    // groups
    /// Source: keycloak/keycloak@v22.0.0
    ///         services/src/main/java/org/keycloak/services/resources/admin/GroupsResource.java
    async fn list_groups(&self, realm: &str) -> Result<Vec<Group>, ClientError> {
        self.get_json(&format!("/admin/realms/{realm}/groups")).await
    }
    async fn get_group(&self, realm: &str, id: &str) -> Result<Group, ClientError> {
        self.get_json(&format!("/admin/realms/{realm}/groups/{id}")).await
    }
    async fn create_group(&self, realm: &str, group: &Group) -> Result<Group, ClientError> {
        self.post_json(&format!("/admin/realms/{realm}/groups"), group).await
    }
    async fn update_group(&self, realm: &str, group: &Group) -> Result<Group, ClientError> {
        self.put_json(&format!("/admin/realms/{realm}/groups/{}", group.id), group)
            .await
    }
    async fn delete_group(&self, realm: &str, id: &str) -> Result<(), ClientError> {
        self.delete_path(&format!("/admin/realms/{realm}/groups/{id}")).await
    }

    // identity providers
    /// Source: keycloak/keycloak@v22.0.0
    ///         services/src/main/java/org/keycloak/services/resources/admin/IdentityProvidersResource.java
    async fn list_identity_providers(
        &self,
        realm: &str,
    ) -> Result<Vec<IdentityProvider>, ClientError> {
        self.get_json(&format!(
            "/admin/realms/{realm}/identity-provider/instances"
        ))
        .await
    }
    async fn get_identity_provider(
        &self,
        realm: &str,
        alias: &str,
    ) -> Result<IdentityProvider, ClientError> {
        self.get_json(&format!(
            "/admin/realms/{realm}/identity-provider/instances/{alias}"
        ))
        .await
    }
    async fn create_identity_provider(
        &self,
        realm: &str,
        idp: &IdentityProvider,
    ) -> Result<IdentityProvider, ClientError> {
        self.post_json(
            &format!("/admin/realms/{realm}/identity-provider/instances"),
            idp,
        )
        .await
    }
    async fn update_identity_provider(
        &self,
        realm: &str,
        idp: &IdentityProvider,
    ) -> Result<IdentityProvider, ClientError> {
        self.put_json(
            &format!(
                "/admin/realms/{realm}/identity-provider/instances/{}",
                idp.alias
            ),
            idp,
        )
        .await
    }
    async fn delete_identity_provider(
        &self,
        realm: &str,
        alias: &str,
    ) -> Result<(), ClientError> {
        self.delete_path(&format!(
            "/admin/realms/{realm}/identity-provider/instances/{alias}"
        ))
        .await
    }

    // auth flows
    /// Source: keycloak/keycloak@v22.0.0
    ///         services/src/main/java/org/keycloak/services/resources/admin/AuthenticationManagementResource.java
    async fn list_auth_flows(&self, realm: &str) -> Result<Vec<AuthFlow>, ClientError> {
        self.get_json(&format!("/admin/realms/{realm}/authentication/flows"))
            .await
    }
    async fn get_auth_flow(&self, realm: &str, id: &str) -> Result<AuthFlow, ClientError> {
        self.get_json(&format!(
            "/admin/realms/{realm}/authentication/flows/{id}"
        ))
        .await
    }
    async fn create_auth_flow(
        &self,
        realm: &str,
        flow: &AuthFlow,
    ) -> Result<AuthFlow, ClientError> {
        self.post_json(
            &format!("/admin/realms/{realm}/authentication/flows"),
            flow,
        )
        .await
    }
    async fn update_auth_flow(
        &self,
        realm: &str,
        flow: &AuthFlow,
    ) -> Result<AuthFlow, ClientError> {
        self.put_json(
            &format!("/admin/realms/{realm}/authentication/flows/{}", flow.id),
            flow,
        )
        .await
    }
    async fn delete_auth_flow(&self, realm: &str, id: &str) -> Result<(), ClientError> {
        self.delete_path(&format!(
            "/admin/realms/{realm}/authentication/flows/{id}"
        ))
        .await
    }

    // events
    /// Source: keycloak/keycloak@v22.0.0
    ///         services/src/main/java/org/keycloak/services/resources/admin/RealmAdminResource.java#getAdminEvents
    async fn list_events(
        &self,
        realm: &str,
        since: Option<i64>,
        kinds: &[&str],
    ) -> Result<Vec<EventPayload>, ClientError> {
        let mut url = format!("/admin/realms/{realm}/events");
        let mut params: Vec<(String, String)> = Vec::new();
        if let Some(t) = since {
            params.push(("dateFrom".into(), t.to_string()));
        }
        for k in kinds {
            params.push(("type".into(), (*k).to_string()));
        }
        if !params.is_empty() {
            let q: Vec<String> = params
                .into_iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect();
            url.push('?');
            url.push_str(&q.join("&"));
        }
        self.get_json(&url).await
    }

    // sessions
    async fn list_sessions(&self, realm: &str) -> Result<Vec<UserSession>, ClientError> {
        self.get_json(&format!("/admin/realms/{realm}/sessions"))
            .await
    }
    async fn delete_session(&self, realm: &str, id: &str) -> Result<(), ClientError> {
        self.delete_path(&format!("/admin/realms/{realm}/sessions/{id}"))
            .await
    }

    // account console
    /// Source: keycloak/keycloak@v22.0.0
    ///         services/src/main/java/org/keycloak/services/resources/account/AccountRestService.java
    async fn get_account_profile(
        &self,
        realm: &str,
        username: &str,
    ) -> Result<AccountProfile, ClientError> {
        self.get_json(&format!("/realms/{realm}/account?username={username}"))
            .await
    }
    async fn update_account_profile(
        &self,
        realm: &str,
        _username: &str,
        profile: &AccountProfile,
    ) -> Result<AccountProfile, ClientError> {
        self.put_json(&format!("/realms/{realm}/account"), profile).await
    }
    async fn list_account_sessions(
        &self,
        realm: &str,
        username: &str,
    ) -> Result<Vec<UserSession>, ClientError> {
        self.get_json(&format!(
            "/realms/{realm}/account/sessions?username={username}"
        ))
        .await
    }
    async fn list_account_credentials(
        &self,
        realm: &str,
        username: &str,
    ) -> Result<Vec<Credential>, ClientError> {
        self.get_json(&format!(
            "/realms/{realm}/account/credentials?username={username}"
        ))
        .await
    }
    async fn list_account_applications(
        &self,
        realm: &str,
        username: &str,
    ) -> Result<Vec<LinkedApplication>, ClientError> {
        self.get_json(&format!(
            "/realms/{realm}/account/applications?username={username}"
        ))
        .await
    }
    async fn list_account_federated_identities(
        &self,
        realm: &str,
        username: &str,
    ) -> Result<Vec<FederatedIdentity>, ClientError> {
        self.get_json(&format!(
            "/realms/{realm}/account/linked-accounts?username={username}"
        ))
        .await
    }

    /// Subscribe to a Server-Sent-Events stream of admin events.
    ///
    /// The cave-auth-side SSE endpoint at `/admin/realms/{realm}/events/stream`
    /// emits one JSON object per `data:` line. This impl spawns a task
    /// that reads the response body line-by-line and forwards parsed
    /// events into an mpsc channel; the caller awaits the receiver.
    async fn subscribe_events(
        &self,
        realm: &str,
    ) -> Result<tokio::sync::mpsc::Receiver<EventPayload>, ClientError> {
        let url = self.url(&format!("/admin/realms/{realm}/events/stream"));
        let resp = self
            .auth(
                self.client
                    .get(&url)
                    .header("accept", "text/event-stream"),
            )
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(ClientError::from_status(status, body));
        }
        let (tx, rx) = tokio::sync::mpsc::channel(64);
        tokio::spawn(async move {
            let mut resp = resp;
            let mut buf = String::new();
            // Read the body in chunks. `Response::chunk()` is available
            // without the optional `stream` feature on reqwest; each
            // chunk is parsed line-by-line as Server-Sent-Events.
            while let Ok(Some(bytes)) = resp.chunk().await {
                let Ok(text) = std::str::from_utf8(&bytes) else { continue };
                buf.push_str(text);
                while let Some(idx) = buf.find('\n') {
                    let line = buf[..idx].trim_start_matches("data:").trim().to_string();
                    buf = buf[idx + 1..].to_string();
                    if line.is_empty() {
                        continue;
                    }
                    if let Ok(evt) = serde_json::from_str::<EventPayload>(&line) {
                        if tx.send(evt).await.is_err() {
                            return;
                        }
                    }
                }
            }
        });
        Ok(rx)
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;

    fn realm_acme() -> Realm {
        Realm {
            id: "acme-realm".into(),
            display_name: "Acme".into(),
            enabled: true,
            ssl_required: "external".into(),
            access_token_lifespan: 300,
        }
    }

    // ── AuthMockClient ────────────────────────────────────────────────

    #[tokio::test]
    async fn mock_create_realm_persists() {
        let c = AuthMockClient::new();
        let r = c.create_realm(&realm_acme()).await.unwrap();
        assert_eq!(r.id, "acme-realm");
        let listed = c.list_realms().await.unwrap();
        assert_eq!(listed.len(), 1);
    }

    #[tokio::test]
    async fn mock_create_realm_conflict_on_duplicate() {
        let c = AuthMockClient::new();
        c.create_realm(&realm_acme()).await.unwrap();
        let err = c.create_realm(&realm_acme()).await.unwrap_err();
        assert!(matches!(err, ClientError::Conflict(_)));
    }

    #[tokio::test]
    async fn mock_update_realm_returns_not_found_for_unknown() {
        let c = AuthMockClient::new();
        let err = c.update_realm(&realm_acme()).await.unwrap_err();
        assert!(matches!(err, ClientError::NotFound(_)));
    }

    #[tokio::test]
    async fn mock_delete_then_get_returns_not_found() {
        let c = AuthMockClient::new();
        c.create_realm(&realm_acme()).await.unwrap();
        c.delete_realm("acme-realm").await.unwrap();
        let err = c.get_realm("acme-realm").await.unwrap_err();
        assert!(matches!(err, ClientError::NotFound(_)));
    }

    #[tokio::test]
    async fn mock_clients_crud_isolates_per_realm() {
        let c = AuthMockClient::new();
        let mut a = ClientApp {
            id: "c1".into(),
            client_id: "portal".into(),
            ..Default::default()
        };
        c.create_client("acme-realm", &a).await.unwrap();
        a.id = "c2".into();
        a.client_id = "portal".into();
        c.create_client("other-realm", &a).await.unwrap();
        assert_eq!(c.list_clients("acme-realm").await.unwrap().len(), 1);
        assert_eq!(c.list_clients("other-realm").await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn mock_users_listing_is_sorted_by_username() {
        let c = AuthMockClient::new();
        c.create_user(
            "acme-realm",
            &User {
                id: "u-b".into(),
                username: "bob".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        c.create_user(
            "acme-realm",
            &User {
                id: "u-a".into(),
                username: "alice".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let users = c.list_users("acme-realm").await.unwrap();
        assert_eq!(users[0].username, "alice");
        assert_eq!(users[1].username, "bob");
    }

    #[tokio::test]
    async fn mock_roles_create_and_delete_round_trip() {
        let c = AuthMockClient::new();
        let r = Role {
            id: "r-1".into(),
            name: "platform_admin".into(),
            ..Default::default()
        };
        c.create_role("acme-realm", &r).await.unwrap();
        c.delete_role("acme-realm", "platform_admin").await.unwrap();
        assert!(c.list_roles("acme-realm").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn mock_groups_update_overwrites_fields() {
        let c = AuthMockClient::new();
        let g = Group {
            id: "g-1".into(),
            name: "engineering".into(),
            path: "/engineering".into(),
            ..Default::default()
        };
        c.create_group("acme-realm", &g).await.unwrap();
        let mut g2 = g.clone();
        g2.name = "Engineering".into();
        c.update_group("acme-realm", &g2).await.unwrap();
        let got = c.get_group("acme-realm", "g-1").await.unwrap();
        assert_eq!(got.name, "Engineering");
    }

    #[tokio::test]
    async fn mock_idp_filters_by_realm() {
        let c = AuthMockClient::new();
        c.create_identity_provider(
            "acme-realm",
            &IdentityProvider {
                alias: "github".into(),
                provider_id: "github".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        c.create_identity_provider(
            "other-realm",
            &IdentityProvider {
                alias: "saml".into(),
                provider_id: "saml".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let acme = c.list_identity_providers("acme-realm").await.unwrap();
        assert_eq!(acme.len(), 1);
        assert_eq!(acme[0].alias, "github");
    }

    #[tokio::test]
    async fn mock_auth_flows_built_in_flag_round_trips() {
        let c = AuthMockClient::new();
        c.create_auth_flow(
            "acme-realm",
            &AuthFlow {
                id: "browser".into(),
                alias: "browser".into(),
                built_in: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let f = c.get_auth_flow("acme-realm", "browser").await.unwrap();
        assert!(f.built_in);
    }

    #[tokio::test]
    async fn mock_events_filtered_by_kind_and_since() {
        let c = AuthMockClient::new();
        c.push_event(EventPayload {
            time: 100,
            realm: "acme-realm".into(),
            kind: "LOGIN".into(),
            ..Default::default()
        });
        c.push_event(EventPayload {
            time: 200,
            realm: "acme-realm".into(),
            kind: "LOGIN_ERROR".into(),
            ..Default::default()
        });
        c.push_event(EventPayload {
            time: 300,
            realm: "other-realm".into(),
            kind: "LOGIN".into(),
            ..Default::default()
        });
        let only_acme = c.list_events("acme-realm", None, &[]).await.unwrap();
        assert_eq!(only_acme.len(), 2);
        let only_errors = c
            .list_events("acme-realm", None, &["LOGIN_ERROR"])
            .await
            .unwrap();
        assert_eq!(only_errors.len(), 1);
        let recent = c.list_events("acme-realm", Some(150), &[]).await.unwrap();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].time, 200);
    }

    // ── AuthApiClient (httpmock) ─────────────────────────────────────

    #[tokio::test]
    async fn api_list_realms_parses_array() {
        let server = MockServer::start();
        let _m = server.mock(|when, then| {
            when.method(GET).path("/admin/realms");
            then.status(200)
                .header("content-type", "application/json")
                .body(r#"[{"id":"acme-realm","displayName":"Acme","enabled":true}]"#);
        });
        let c = AuthApiClient::test_against(server.base_url());
        let v = c.list_realms().await.unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].id, "acme-realm");
        assert_eq!(v[0].display_name, "Acme");
    }

    #[tokio::test]
    async fn api_get_realm_404_maps_to_not_found() {
        let server = MockServer::start();
        let _m = server.mock(|when, then| {
            when.method(GET).path("/admin/realms/missing");
            then.status(404).body("realm not found");
        });
        let c = AuthApiClient::test_against(server.base_url());
        let err = c.get_realm("missing").await.unwrap_err();
        assert!(matches!(err, ClientError::NotFound(_)));
    }

    #[tokio::test]
    async fn api_create_realm_409_maps_to_conflict() {
        let server = MockServer::start();
        let _m = server.mock(|when, then| {
            when.method(POST).path("/admin/realms");
            then.status(409).body("realm_exists");
        });
        let c = AuthApiClient::test_against(server.base_url());
        let err = c.create_realm(&realm_acme()).await.unwrap_err();
        assert!(matches!(err, ClientError::Conflict(_)));
    }

    #[tokio::test]
    async fn api_create_user_round_trip_via_post() {
        let server = MockServer::start();
        let _m = server.mock(|when, then| {
            when.method(POST).path("/admin/realms/acme-realm/users");
            then.status(201)
                .header("content-type", "application/json")
                .body(r#"{"id":"u-1","username":"alice","enabled":true}"#);
        });
        let c = AuthApiClient::test_against(server.base_url());
        let got = c
            .create_user(
                "acme-realm",
                &User {
                    id: "u-1".into(),
                    username: "alice".into(),
                    enabled: true,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(got.username, "alice");
    }

    #[tokio::test]
    async fn api_list_users_returns_decoded_users() {
        let server = MockServer::start();
        let _m = server.mock(|when, then| {
            when.method(GET).path("/admin/realms/acme-realm/users");
            then.status(200)
                .header("content-type", "application/json")
                .body(
                    r#"[{"id":"u-1","username":"alice"},{"id":"u-2","username":"bob"}]"#,
                );
        });
        let c = AuthApiClient::test_against(server.base_url());
        let v = c.list_users("acme-realm").await.unwrap();
        assert_eq!(v.len(), 2);
        assert!(v.iter().any(|u| u.username == "alice"));
        assert!(v.iter().any(|u| u.username == "bob"));
    }

    #[tokio::test]
    async fn api_update_role_uses_put_at_named_path() {
        let server = MockServer::start();
        let _m = server.mock(|when, then| {
            when.method(PUT).path("/admin/realms/acme-realm/roles/dev");
            then.status(200)
                .header("content-type", "application/json")
                .body(r#"{"id":"r","name":"dev","description":"updated"}"#);
        });
        let c = AuthApiClient::test_against(server.base_url());
        let r = c
            .update_role(
                "acme-realm",
                &Role {
                    id: "r".into(),
                    name: "dev".into(),
                    description: Some("updated".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(r.description.as_deref(), Some("updated"));
    }

    #[tokio::test]
    async fn api_delete_client_returns_unit_on_204() {
        let server = MockServer::start();
        let _m = server.mock(|when, then| {
            when.method(DELETE).path("/admin/realms/acme-realm/clients/c-1");
            then.status(204);
        });
        let c = AuthApiClient::test_against(server.base_url());
        c.delete_client("acme-realm", "c-1").await.unwrap();
    }

    #[tokio::test]
    async fn api_idp_uses_identity_provider_instances_path() {
        let server = MockServer::start();
        let _m = server.mock(|when, then| {
            when.method(GET)
                .path("/admin/realms/acme-realm/identity-provider/instances");
            then.status(200)
                .header("content-type", "application/json")
                .body(r#"[{"alias":"github","providerId":"github","enabled":true}]"#);
        });
        let c = AuthApiClient::test_against(server.base_url());
        let v = c.list_identity_providers("acme-realm").await.unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].alias, "github");
    }

    #[tokio::test]
    async fn api_list_events_with_since_and_kinds_builds_query() {
        let server = MockServer::start();
        let _m = server.mock(|when, then| {
            when.method(GET)
                .path("/admin/realms/acme-realm/events")
                .query_param("dateFrom", "150")
                .query_param("type", "LOGIN_ERROR");
            then.status(200)
                .header("content-type", "application/json")
                .body(r#"[{"time":200,"realm":"acme-realm","type":"LOGIN_ERROR"}]"#);
        });
        let c = AuthApiClient::test_against(server.base_url());
        let v = c
            .list_events("acme-realm", Some(150), &["LOGIN_ERROR"])
            .await
            .unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].kind, "LOGIN_ERROR");
    }

    #[tokio::test]
    async fn api_get_account_profile_uses_realms_path() {
        let server = MockServer::start();
        let _m = server.mock(|when, then| {
            when.method(GET).path("/realms/acme-realm/account");
            then.status(200)
                .header("content-type", "application/json")
                .body(r#"{"username":"alice","email":"a@acme.io","emailVerified":true}"#);
        });
        let c = AuthApiClient::test_against(server.base_url());
        let p = c.get_account_profile("acme-realm", "alice").await.unwrap();
        assert_eq!(p.username, "alice");
        assert_eq!(p.email.as_deref(), Some("a@acme.io"));
    }

    #[tokio::test]
    async fn api_status_500_maps_to_generic_status() {
        let server = MockServer::start();
        let _m = server.mock(|when, then| {
            when.method(GET).path("/admin/realms");
            then.status(500).body("boom");
        });
        let c = AuthApiClient::test_against(server.base_url());
        let err = c.list_realms().await.unwrap_err();
        match err {
            ClientError::Status { status, .. } => assert_eq!(status.as_u16(), 500),
            other => panic!("expected Status(500), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn api_decode_error_on_garbage_json() {
        let server = MockServer::start();
        let _m = server.mock(|when, then| {
            when.method(GET).path("/admin/realms");
            then.status(200).body("not-json");
        });
        let c = AuthApiClient::test_against(server.base_url());
        let err = c.list_realms().await.unwrap_err();
        assert!(matches!(err, ClientError::Decode(_)));
    }

    #[tokio::test]
    async fn mock_subscribe_events_delivers_realm_events() {
        let c = AuthMockClient::new();
        let mut rx = c.subscribe_events("acme-realm").await.unwrap();
        // Subscribe is realm-scoped; push from another realm must not appear.
        let evt = EventPayload {
            time: 1,
            realm: "acme-realm".into(),
            kind: "LOGIN".into(),
            ..Default::default()
        };
        c.push_event(evt.clone());
        c.push_event(EventPayload {
            time: 2,
            realm: "other-realm".into(),
            kind: "LOGIN".into(),
            ..Default::default()
        });
        // The first matching push should arrive; the non-matching one is filtered.
        let got = tokio::time::timeout(Duration::from_millis(500), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(got.realm, "acme-realm");
    }

    #[tokio::test]
    async fn mock_account_profile_round_trip() {
        let c = AuthMockClient::new();
        c.seed_profile(
            "acme-realm",
            AccountProfile {
                username: "alice".into(),
                email: Some("alice@acme.io".into()),
                ..Default::default()
            },
        );
        let p = c.get_account_profile("acme-realm", "alice").await.unwrap();
        assert_eq!(p.email.as_deref(), Some("alice@acme.io"));
        let updated = AccountProfile {
            username: "alice".into(),
            email: Some("alice2@acme.io".into()),
            first_name: Some("Alice".into()),
            ..Default::default()
        };
        let p2 = c
            .update_account_profile("acme-realm", "alice", &updated)
            .await
            .unwrap();
        assert_eq!(p2.first_name.as_deref(), Some("Alice"));
    }

    #[tokio::test]
    async fn mock_account_sessions_filter_by_username() {
        let c = AuthMockClient::new();
        c.seed_session(UserSession {
            id: "s-1".into(),
            realm: "acme-realm".into(),
            username: "alice".into(),
            start: 100,
            ..Default::default()
        });
        c.seed_session(UserSession {
            id: "s-2".into(),
            realm: "acme-realm".into(),
            username: "bob".into(),
            start: 101,
            ..Default::default()
        });
        let alice_sessions = c
            .list_account_sessions("acme-realm", "alice")
            .await
            .unwrap();
        assert_eq!(alice_sessions.len(), 1);
        assert_eq!(alice_sessions[0].id, "s-1");
    }

    #[tokio::test]
    async fn mock_account_credentials_returns_empty_when_unseen() {
        let c = AuthMockClient::new();
        let creds = c
            .list_account_credentials("acme-realm", "ghost")
            .await
            .unwrap();
        assert!(creds.is_empty());
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 model/jpa/src/main/java/org/keycloak/models/jpa/entities/
//
// Port of Keycloak JPA `*Entity` classes (`RealmEntity`, `UserEntity`,
// `ClientEntity`, `RoleEntity`, `GroupEntity`, `IdentityProviderEntity`,
// `AuthenticationFlowEntity`). Each entity preserves the upstream column
// shape so a side-by-side schema diff against the Liquibase changelog
// (`keycloak-model-jpa-changelog-master.xml`) stays trivial.

//! Plain Rust structs representing the persisted shape of every
//! [`PersistenceBackend`](super::backend::PersistenceBackend) entity.
//!
//! Every entity carries the trio `created_at`, `updated_at`, `deleted_at`
//! so that queries can default to filtering out soft-deleted rows the
//! way Keycloak's JPA layer filters by a `deleted` column on tombstones.
//!
//! UUID v4 is used for primary keys to keep the schema independent of any
//! sequence in the underlying RDBMS.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

/// Common audit columns embedded in every entity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditFields {
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl AuditFields {
    pub fn new_now() -> Self {
        let now = Utc::now();
        Self {
            created_at: now,
            updated_at: now,
            deleted_at: None,
        }
    }

    pub fn is_deleted(&self) -> bool {
        self.deleted_at.is_some()
    }
}

/// Keycloak `RealmEntity` — the top-level tenant boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RealmEntity {
    pub id: Uuid,
    pub name: String,
    pub display_name: Option<String>,
    pub enabled: bool,
    pub ssl_required: String,       // "external" | "all" | "none"
    pub access_token_lifespan: i32, // seconds
    pub sso_session_idle_timeout: i32,
    pub sso_session_max_lifespan: i32,
    pub registration_allowed: bool,
    pub remember_me: bool,
    pub verify_email: bool,
    pub login_with_email_allowed: bool,
    pub duplicate_emails_allowed: bool,
    pub reset_password_allowed: bool,
    pub edit_username_allowed: bool,
    pub brute_force_protected: bool,
    pub audit: AuditFields,
}

impl RealmEntity {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            display_name: None,
            enabled: true,
            ssl_required: "external".to_string(),
            access_token_lifespan: 300,
            sso_session_idle_timeout: 1800,
            sso_session_max_lifespan: 36_000,
            registration_allowed: false,
            remember_me: false,
            verify_email: false,
            login_with_email_allowed: true,
            duplicate_emails_allowed: false,
            reset_password_allowed: false,
            edit_username_allowed: false,
            brute_force_protected: false,
            audit: AuditFields::new_now(),
        }
    }
}

/// Keycloak `UserEntity` plus inlined credentials + attributes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserEntity {
    pub id: Uuid,
    pub realm_id: Uuid,
    pub username: String,
    pub email: Option<String>,
    pub email_verified: bool,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub enabled: bool,
    pub federation_link: Option<String>,
    pub service_account_client_link: Option<Uuid>,
    pub credentials: Vec<UserCredential>,
    pub attributes: BTreeMap<String, Vec<String>>,
    pub audit: AuditFields,
}

impl UserEntity {
    pub fn new(realm_id: Uuid, username: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            realm_id,
            username: username.into(),
            email: None,
            email_verified: false,
            first_name: None,
            last_name: None,
            enabled: true,
            federation_link: None,
            service_account_client_link: None,
            credentials: Vec::new(),
            attributes: BTreeMap::new(),
            audit: AuditFields::new_now(),
        }
    }
}

/// `CredentialEntity` row associated with a user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserCredential {
    pub id: Uuid,
    pub credential_type: String, // "password" | "otp" | "webauthn" | ...
    pub secret_data: String,     // JSON: salt, hash, iter count, ...
    pub credential_data: String, // JSON: algorithm, hashLength, ...
    pub priority: i32,
}

/// Keycloak `ClientEntity`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientEntity {
    pub id: Uuid,
    pub realm_id: Uuid,
    pub client_id: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub enabled: bool,
    pub client_authenticator_type: String, // "client-secret" | "client-jwt" | ...
    pub secret: Option<String>,
    pub redirect_uris: Vec<String>,
    pub web_origins: Vec<String>,
    pub bearer_only: bool,
    pub consent_required: bool,
    pub standard_flow_enabled: bool,
    pub implicit_flow_enabled: bool,
    pub direct_access_grants_enabled: bool,
    pub service_accounts_enabled: bool,
    pub public_client: bool,
    pub frontchannel_logout: bool,
    pub protocol: String, // "openid-connect" | "saml"
    pub attributes: BTreeMap<String, String>,
    pub audit: AuditFields,
}

impl ClientEntity {
    pub fn new(realm_id: Uuid, client_id: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            realm_id,
            client_id: client_id.into(),
            name: None,
            description: None,
            enabled: true,
            client_authenticator_type: "client-secret".to_string(),
            secret: None,
            redirect_uris: Vec::new(),
            web_origins: Vec::new(),
            bearer_only: false,
            consent_required: false,
            standard_flow_enabled: true,
            implicit_flow_enabled: false,
            direct_access_grants_enabled: false,
            service_accounts_enabled: false,
            public_client: false,
            frontchannel_logout: false,
            protocol: "openid-connect".to_string(),
            attributes: BTreeMap::new(),
            audit: AuditFields::new_now(),
        }
    }
}

/// Keycloak `RoleEntity` (realm-scoped or client-scoped).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleEntity {
    pub id: Uuid,
    pub realm_id: Uuid,
    pub client_id: Option<Uuid>, // None = realm role
    pub name: String,
    pub description: Option<String>,
    pub composite: bool,
    pub attributes: BTreeMap<String, Vec<String>>,
    pub audit: AuditFields,
}

impl RoleEntity {
    pub fn new(realm_id: Uuid, name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            realm_id,
            client_id: None,
            name: name.into(),
            description: None,
            composite: false,
            attributes: BTreeMap::new(),
            audit: AuditFields::new_now(),
        }
    }
}

/// Keycloak `GroupEntity`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupEntity {
    pub id: Uuid,
    pub realm_id: Uuid,
    pub parent_id: Option<Uuid>,
    pub name: String,
    pub attributes: BTreeMap<String, Vec<String>>,
    pub role_ids: Vec<Uuid>,
    pub audit: AuditFields,
}

impl GroupEntity {
    pub fn new(realm_id: Uuid, name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            realm_id,
            parent_id: None,
            name: name.into(),
            attributes: BTreeMap::new(),
            role_ids: Vec::new(),
            audit: AuditFields::new_now(),
        }
    }
}

/// Keycloak `IdentityProviderEntity` — federated IdP config.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdentityProviderEntity {
    pub id: Uuid,
    pub realm_id: Uuid,
    pub alias: String,
    pub display_name: Option<String>,
    pub provider_id: String, // "saml" | "oidc" | "google" | "github" | ...
    pub enabled: bool,
    pub trust_email: bool,
    pub store_token: bool,
    pub add_read_token_role_on_create: bool,
    pub authenticate_by_default: bool,
    pub link_only: bool,
    pub first_broker_login_flow_id: Option<Uuid>,
    pub post_broker_login_flow_id: Option<Uuid>,
    pub config: BTreeMap<String, String>,
    pub mappers: Vec<IdpMapper>,
    pub audit: AuditFields,
}

impl IdentityProviderEntity {
    pub fn new(realm_id: Uuid, alias: impl Into<String>, provider_id: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            realm_id,
            alias: alias.into(),
            display_name: None,
            provider_id: provider_id.into(),
            enabled: true,
            trust_email: false,
            store_token: false,
            add_read_token_role_on_create: false,
            authenticate_by_default: false,
            link_only: false,
            first_broker_login_flow_id: None,
            post_broker_login_flow_id: None,
            config: BTreeMap::new(),
            mappers: Vec::new(),
            audit: AuditFields::new_now(),
        }
    }
}

/// `IdentityProviderMapperEntity` row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdpMapper {
    pub id: Uuid,
    pub name: String,
    pub mapper_type: String, // "hardcoded-attribute" | "user-attribute" | ...
    pub config: BTreeMap<String, String>,
}

/// Keycloak `AuthenticationFlowEntity` + inlined executions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthFlowEntity {
    pub id: Uuid,
    pub realm_id: Uuid,
    pub alias: String,
    pub description: Option<String>,
    pub provider_id: String, // "basic-flow" | "client-flow"
    pub top_level: bool,
    pub built_in: bool,
    pub executions: Vec<FlowExecution>,
    pub audit: AuditFields,
}

impl AuthFlowEntity {
    pub fn new(realm_id: Uuid, alias: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            realm_id,
            alias: alias.into(),
            description: None,
            provider_id: "basic-flow".to_string(),
            top_level: true,
            built_in: false,
            executions: Vec::new(),
            audit: AuditFields::new_now(),
        }
    }
}

/// `AuthenticationExecutionEntity` row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlowExecution {
    pub id: Uuid,
    pub authenticator: String, // "auth-username-password-form" | ...
    pub requirement: String,   // "REQUIRED" | "ALTERNATIVE" | "DISABLED" | "CONDITIONAL"
    pub priority: i32,
    pub authenticator_config: BTreeMap<String, String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn realm_entity_new_defaults() {
        let r = RealmEntity::new("master");
        assert_eq!(r.name, "master");
        assert!(r.enabled);
        assert_eq!(r.access_token_lifespan, 300);
        assert!(!r.audit.is_deleted());
    }

    #[test]
    fn audit_fields_new_now_timestamps_match() {
        let a = AuditFields::new_now();
        assert_eq!(a.created_at, a.updated_at);
        assert!(a.deleted_at.is_none());
        assert!(!a.is_deleted());
    }

    #[test]
    fn audit_is_deleted_when_tombstoned() {
        let mut a = AuditFields::new_now();
        a.deleted_at = Some(Utc::now());
        assert!(a.is_deleted());
    }

    #[test]
    fn user_entity_carries_realm_fk() {
        let realm = RealmEntity::new("r1");
        let u = UserEntity::new(realm.id, "alice");
        assert_eq!(u.realm_id, realm.id);
        assert_eq!(u.username, "alice");
        assert!(u.enabled);
        assert!(u.credentials.is_empty());
    }

    #[test]
    fn client_entity_default_protocol_is_openid_connect() {
        let realm = RealmEntity::new("r2");
        let c = ClientEntity::new(realm.id, "frontend");
        assert_eq!(c.protocol, "openid-connect");
        assert!(c.standard_flow_enabled);
        assert!(!c.public_client);
    }

    #[test]
    fn role_realm_vs_client_distinction() {
        let realm = RealmEntity::new("r3");
        let realm_role = RoleEntity::new(realm.id, "admin");
        assert!(
            realm_role.client_id.is_none(),
            "realm role has no client_id"
        );

        let client = ClientEntity::new(realm.id, "api");
        let mut client_role = RoleEntity::new(realm.id, "viewer");
        client_role.client_id = Some(client.id);
        assert_eq!(client_role.client_id, Some(client.id));
    }

    #[test]
    fn group_supports_nesting() {
        let realm = RealmEntity::new("r4");
        let parent = GroupEntity::new(realm.id, "engineering");
        let mut child = GroupEntity::new(realm.id, "backend");
        child.parent_id = Some(parent.id);
        assert_eq!(child.parent_id, Some(parent.id));
    }

    #[test]
    fn idp_entity_holds_mappers() {
        let realm = RealmEntity::new("r5");
        let mut idp = IdentityProviderEntity::new(realm.id, "okta-prod", "oidc");
        idp.mappers.push(IdpMapper {
            id: Uuid::new_v4(),
            name: "email-mapper".to_string(),
            mapper_type: "user-attribute".to_string(),
            config: BTreeMap::new(),
        });
        assert_eq!(idp.mappers.len(), 1);
        assert_eq!(idp.provider_id, "oidc");
    }

    #[test]
    fn auth_flow_holds_executions() {
        let realm = RealmEntity::new("r6");
        let mut f = AuthFlowEntity::new(realm.id, "browser");
        f.executions.push(FlowExecution {
            id: Uuid::new_v4(),
            authenticator: "auth-username-password-form".to_string(),
            requirement: "REQUIRED".to_string(),
            priority: 10,
            authenticator_config: BTreeMap::new(),
        });
        assert_eq!(f.executions.len(), 1);
        assert!(f.top_level);
    }

    #[test]
    fn entity_serde_roundtrip() {
        let realm = RealmEntity::new("rserde");
        let json = serde_json::to_string(&realm).unwrap();
        let back: RealmEntity = serde_json::from_str(&json).unwrap();
        assert_eq!(realm, back);
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@b825ba97 services/src/main/java/org/keycloak/services/resources/admin/IdentityProviderResource.java
// Source: keycloak/keycloak@b825ba97 services/src/main/java/org/keycloak/services/resources/admin/IdentityProvidersResource.java
//
//! Identity Provider REST CRUD.
//!
//! Routes:
//!   GET    /admin/realms/{realm}/identity-provider/instances
//!   POST   /admin/realms/{realm}/identity-provider/instances
//!   GET    /admin/realms/{realm}/identity-provider/instances/{alias}
//!   PUT    /admin/realms/{realm}/identity-provider/instances/{alias}
//!   DELETE /admin/realms/{realm}/identity-provider/instances/{alias}
//!   GET    /admin/realms/{realm}/identity-provider/providers
//!   POST   /admin/realms/{realm}/identity-provider/import-config

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Mirrors org.keycloak.representations.idm.IdentityProviderRepresentation.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IdentityProviderModel {
    pub alias: String,
    #[serde(default)]
    pub internal_id: Option<String>,
    pub provider_id: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub trust_email: bool,
    #[serde(default)]
    pub store_token: bool,
    #[serde(default)]
    pub add_read_token_role_on_create: bool,
    #[serde(default)]
    pub authenticate_by_default: bool,
    #[serde(default)]
    pub link_only: bool,
    #[serde(default)]
    pub first_broker_login_flow_alias: Option<String>,
    #[serde(default)]
    pub post_broker_login_flow_alias: Option<String>,
    #[serde(default)]
    pub config: HashMap<String, String>,
}

fn default_true() -> bool { true }

/// Built-in factory list — Keycloak ships ≈20 factories; we list the popular
/// social + protocol-level providers + the keystone OIDC/SAML pair.
#[derive(Debug, Clone, Serialize)]
pub struct IdpFactoryDescriptor {
    pub id: &'static str,
    pub name: &'static str,
    pub group_name: &'static str,
}

pub const FACTORIES: &[IdpFactoryDescriptor] = &[
    IdpFactoryDescriptor { id: "oidc",     name: "OpenID Connect v1.0",                group_name: "Custom" },
    IdpFactoryDescriptor { id: "saml",     name: "SAML v2.0",                          group_name: "Custom" },
    IdpFactoryDescriptor { id: "google",   name: "Google",                             group_name: "Social" },
    IdpFactoryDescriptor { id: "github",   name: "GitHub",                             group_name: "Social" },
    IdpFactoryDescriptor { id: "facebook", name: "Facebook",                           group_name: "Social" },
    IdpFactoryDescriptor { id: "microsoft", name: "Microsoft",                         group_name: "Social" },
    IdpFactoryDescriptor { id: "apple",    name: "Apple",                              group_name: "Social" },
    IdpFactoryDescriptor { id: "linkedin", name: "LinkedIn",                           group_name: "Social" },
    IdpFactoryDescriptor { id: "twitter",  name: "Twitter / X",                        group_name: "Social" },
    IdpFactoryDescriptor { id: "okta",     name: "Okta",                               group_name: "Social" },
    IdpFactoryDescriptor { id: "gitlab",   name: "GitLab",                             group_name: "Social" },
    IdpFactoryDescriptor { id: "bitbucket",name: "Bitbucket",                          group_name: "Social" },
    IdpFactoryDescriptor { id: "instagram", name: "Instagram",                         group_name: "Social" },
    IdpFactoryDescriptor { id: "paypal",   name: "PayPal",                             group_name: "Social" },
    IdpFactoryDescriptor { id: "stackoverflow", name: "Stack Overflow",                group_name: "Social" },
    IdpFactoryDescriptor { id: "openshift-v3", name: "OpenShift v3",                   group_name: "Social" },
    IdpFactoryDescriptor { id: "openshift-v4", name: "OpenShift v4",                   group_name: "Social" },
];

#[derive(Clone)]
pub struct IdpService {
    /// (realm, alias) → model.
    inner: Arc<RwLock<HashMap<(String, String), IdentityProviderModel>>>,
}

impl IdpService {
    pub fn new() -> Self { Self { inner: Arc::new(RwLock::new(HashMap::new())) } }
    pub fn cloned(&self) -> Self { self.clone() }

    pub async fn list(&self, realm: &str) -> Vec<IdentityProviderModel> {
        self.inner.read().await.iter()
            .filter(|((r, _), _)| r == realm)
            .map(|(_, v)| v.clone())
            .collect()
    }
    pub async fn get(&self, realm: &str, alias: &str) -> Option<IdentityProviderModel> {
        self.inner.read().await.get(&(realm.to_string(), alias.to_string())).cloned()
    }
    pub async fn create(&self, realm: &str, mut model: IdentityProviderModel) -> Result<IdentityProviderModel, &'static str> {
        if model.alias.is_empty() { return Err("alias_required"); }
        if model.provider_id.is_empty() { return Err("provider_id_required"); }
        let mut store = self.inner.write().await;
        if store.contains_key(&(realm.to_string(), model.alias.clone())) {
            return Err("alias_exists");
        }
        model.internal_id = Some(format!("idp-{}", uuid::Uuid::new_v4()));
        store.insert((realm.to_string(), model.alias.clone()), model.clone());
        Ok(model)
    }
    pub async fn update(&self, realm: &str, alias: &str, model: IdentityProviderModel) -> Result<IdentityProviderModel, &'static str> {
        let mut store = self.inner.write().await;
        let key = (realm.to_string(), alias.to_string());
        let prev = store.get(&key).ok_or("not_found")?.clone();
        let mut merged = model;
        merged.alias = alias.to_string();
        merged.internal_id = prev.internal_id;
        store.insert(key, merged.clone());
        Ok(merged)
    }
    pub async fn delete(&self, realm: &str, alias: &str) -> bool {
        self.inner.write().await.remove(&(realm.to_string(), alias.to_string())).is_some()
    }
}

impl Default for IdpService {
    fn default() -> Self { Self::new() }
}

// ─── Wire types ───────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ImportConfigRequest {
    /// "oidc" or "saml".
    pub provider_id: String,
    /// URL of a `.well-known/openid-configuration` (OIDC) or
    /// SAML metadata URL.
    #[serde(default)]
    pub from_url: Option<String>,
    /// Or the raw metadata document.
    #[serde(default)]
    pub from_xml: Option<String>,
    #[serde(default)]
    pub from_json: Option<String>,
}

/// Returns the config map the operator can paste into a `POST /instances` body.
pub fn import_from_oidc(json_doc: &str) -> Result<HashMap<String, String>, &'static str> {
    let v: serde_json::Value = serde_json::from_str(json_doc).map_err(|_| "invalid_json")?;
    let mut out = HashMap::new();
    if let Some(s) = v.get("issuer").and_then(|x| x.as_str()) {
        out.insert("issuer".into(), s.into());
    }
    if let Some(s) = v.get("authorization_endpoint").and_then(|x| x.as_str()) {
        out.insert("authorizationUrl".into(), s.into());
    }
    if let Some(s) = v.get("token_endpoint").and_then(|x| x.as_str()) {
        out.insert("tokenUrl".into(), s.into());
    }
    if let Some(s) = v.get("userinfo_endpoint").and_then(|x| x.as_str()) {
        out.insert("userInfoUrl".into(), s.into());
    }
    if let Some(s) = v.get("jwks_uri").and_then(|x| x.as_str()) {
        out.insert("jwksUrl".into(), s.into());
    }
    if let Some(s) = v.get("end_session_endpoint").and_then(|x| x.as_str()) {
        out.insert("logoutUrl".into(), s.into());
    }
    if out.is_empty() { return Err("no_recognised_fields"); }
    Ok(out)
}

pub fn import_from_saml(xml_doc: &str) -> Result<HashMap<String, String>, &'static str> {
    let mut out = HashMap::new();
    // Very light XPath-free scan — production code uses cave-auth::saml's parser.
    if let Some(idx) = xml_doc.find("entityID=\"") {
        let rest = &xml_doc[idx + "entityID=\"".len()..];
        if let Some(end) = rest.find('"') {
            out.insert("idpEntityId".into(), rest[..end].into());
        }
    }
    if let Some(idx) = xml_doc.find("Location=\"") {
        let rest = &xml_doc[idx + "Location=\"".len()..];
        if let Some(end) = rest.find('"') {
            out.insert("singleSignOnServiceUrl".into(), rest[..end].into());
        }
    }
    if out.is_empty() { return Err("no_recognised_fields"); }
    Ok(out)
}

// ─── HTTP handlers ────────────────────────────────────────────────────────────

pub async fn list_handler(
    State(svc): State<IdpService>,
    Path(realm): Path<String>,
) -> impl IntoResponse {
    super::super::metrics::inc_idp_op("list", "ok");
    Json(svc.list(&realm).await)
}

pub async fn create_handler(
    State(svc): State<IdpService>,
    Path(realm): Path<String>,
    Json(model): Json<IdentityProviderModel>,
) -> impl IntoResponse {
    match svc.create(&realm, model).await {
        Ok(m) => {
            super::super::metrics::inc_idp_op("create", "ok");
            (StatusCode::CREATED, Json(m)).into_response()
        }
        Err(e) => {
            super::super::metrics::inc_idp_op("create", "fail");
            let body = serde_json::json!({"error": e});
            (StatusCode::CONFLICT, Json(body)).into_response()
        }
    }
}

pub async fn get_handler(
    State(svc): State<IdpService>,
    Path((realm, alias)): Path<(String, String)>,
) -> impl IntoResponse {
    match svc.get(&realm, &alias).await {
        Some(m) => {
            super::super::metrics::inc_idp_op("get", "ok");
            (StatusCode::OK, Json(m)).into_response()
        }
        None => {
            super::super::metrics::inc_idp_op("get", "not_found");
            (StatusCode::NOT_FOUND, Json(serde_json::json!({"error":"not_found"}))).into_response()
        }
    }
}

pub async fn update_handler(
    State(svc): State<IdpService>,
    Path((realm, alias)): Path<(String, String)>,
    Json(model): Json<IdentityProviderModel>,
) -> impl IntoResponse {
    match svc.update(&realm, &alias, model).await {
        Ok(m) => {
            super::super::metrics::inc_idp_op("update", "ok");
            (StatusCode::OK, Json(m)).into_response()
        }
        Err(e) => {
            super::super::metrics::inc_idp_op("update", "fail");
            (StatusCode::NOT_FOUND, Json(serde_json::json!({"error":e}))).into_response()
        }
    }
}

pub async fn delete_handler(
    State(svc): State<IdpService>,
    Path((realm, alias)): Path<(String, String)>,
) -> impl IntoResponse {
    if svc.delete(&realm, &alias).await {
        super::super::metrics::inc_idp_op("delete", "ok");
        StatusCode::NO_CONTENT.into_response()
    } else {
        super::super::metrics::inc_idp_op("delete", "not_found");
        (StatusCode::NOT_FOUND, Json(serde_json::json!({"error":"not_found"}))).into_response()
    }
}

pub async fn factories_handler() -> impl IntoResponse {
    super::super::metrics::inc_idp_op("providers", "ok");
    Json(FACTORIES.iter().cloned().collect::<Vec<_>>())
}

pub async fn import_config_handler(
    Path(_realm): Path<String>,
    Json(req): Json<ImportConfigRequest>,
) -> impl IntoResponse {
    let out = match req.provider_id.as_str() {
        "oidc" => {
            if let Some(j) = req.from_json {
                import_from_oidc(&j)
            } else if let Some(_url) = req.from_url {
                // We don't fetch in this layer — return an honest error.
                Err("fetch_not_implemented")
            } else {
                Err("missing_source")
            }
        }
        "saml" => {
            if let Some(x) = req.from_xml {
                import_from_saml(&x)
            } else if let Some(_url) = req.from_url {
                Err("fetch_not_implemented")
            } else {
                Err("missing_source")
            }
        }
        _ => Err("unknown_provider"),
    };
    match out {
        Ok(cfg) => {
            super::super::metrics::inc_idp_op("import_config", "ok");
            (StatusCode::OK, Json(cfg)).into_response()
        }
        Err(e) => {
            super::super::metrics::inc_idp_op("import_config", "fail");
            (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error":e}))).into_response()
        }
    }
}

pub fn router(svc: IdpService) -> Router {
    Router::new()
        .route("/admin/realms/{realm}/identity-provider/instances",
               get(list_handler).post(create_handler))
        .route("/admin/realms/{realm}/identity-provider/instances/{alias}",
               get(get_handler).put(update_handler).delete(delete_handler))
        .route("/admin/realms/{realm}/identity-provider/providers", get(factories_handler))
        .route("/admin/realms/{realm}/identity-provider/import-config", post(import_config_handler))
        .with_state(svc)
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn create_and_get_round_trip() {
        let svc = IdpService::new();
        let m = svc.create("r", IdentityProviderModel {
            alias: "google".into(), provider_id: "google".into(),
            ..Default::default()
        }).await.unwrap();
        assert!(m.internal_id.is_some());
        let got = svc.get("r", "google").await.unwrap();
        assert_eq!(got.alias, "google");
    }

    #[tokio::test]
    async fn create_duplicate_alias_errors() {
        let svc = IdpService::new();
        svc.create("r", IdentityProviderModel { alias: "g".into(), provider_id: "google".into(), ..Default::default() }).await.unwrap();
        let err = svc.create("r", IdentityProviderModel { alias: "g".into(), provider_id: "google".into(), ..Default::default() }).await.unwrap_err();
        assert_eq!(err, "alias_exists");
    }

    #[tokio::test]
    async fn update_preserves_internal_id() {
        let svc = IdpService::new();
        let m = svc.create("r", IdentityProviderModel { alias: "g".into(), provider_id: "google".into(), ..Default::default() }).await.unwrap();
        let id = m.internal_id.clone().unwrap();
        let updated = svc.update("r", "g", IdentityProviderModel {
            alias: "g".into(), provider_id: "google".into(),
            display_name: Some("Google".into()), ..Default::default()
        }).await.unwrap();
        assert_eq!(updated.internal_id.as_deref(), Some(id.as_str()));
        assert_eq!(updated.display_name.as_deref(), Some("Google"));
    }

    #[tokio::test]
    async fn delete_works() {
        let svc = IdpService::new();
        svc.create("r", IdentityProviderModel { alias: "g".into(), provider_id: "google".into(), ..Default::default() }).await.unwrap();
        assert!(svc.delete("r", "g").await);
        assert!(svc.get("r", "g").await.is_none());
    }

    #[tokio::test]
    async fn list_is_realm_scoped() {
        let svc = IdpService::new();
        svc.create("r1", IdentityProviderModel { alias: "g".into(), provider_id: "google".into(), ..Default::default() }).await.unwrap();
        svc.create("r2", IdentityProviderModel { alias: "f".into(), provider_id: "facebook".into(), ..Default::default() }).await.unwrap();
        let r1 = svc.list("r1").await;
        assert_eq!(r1.len(), 1);
        assert_eq!(r1[0].alias, "g");
    }

    #[test]
    fn factories_include_oidc_and_saml() {
        assert!(FACTORIES.iter().any(|f| f.id == "oidc"));
        assert!(FACTORIES.iter().any(|f| f.id == "saml"));
    }

    #[test]
    fn oidc_config_import_picks_endpoints() {
        let doc = r#"{
            "issuer":"https://accounts.google.com",
            "authorization_endpoint":"https://accounts.google.com/o/oauth2/v2/auth",
            "token_endpoint":"https://oauth2.googleapis.com/token",
            "userinfo_endpoint":"https://openidconnect.googleapis.com/v1/userinfo",
            "jwks_uri":"https://www.googleapis.com/oauth2/v3/certs"
        }"#;
        let cfg = import_from_oidc(doc).unwrap();
        assert_eq!(cfg["issuer"], "https://accounts.google.com");
        assert!(cfg.contains_key("authorizationUrl"));
        assert!(cfg.contains_key("tokenUrl"));
        assert!(cfg.contains_key("userInfoUrl"));
        assert!(cfg.contains_key("jwksUrl"));
    }

    #[test]
    fn saml_metadata_import_picks_entity_id() {
        let xml = r#"<md:EntityDescriptor xmlns:md="urn:oasis:names:tc:SAML:2.0:metadata" entityID="https://idp.example.com/saml">
          <md:IDPSSODescriptor>
            <md:SingleSignOnService Binding="urn:oasis:names:tc:SAML:2.0:bindings:HTTP-Redirect" Location="https://idp.example.com/sso"/>
          </md:IDPSSODescriptor>
        </md:EntityDescriptor>"#;
        let cfg = import_from_saml(xml).unwrap();
        assert_eq!(cfg["idpEntityId"], "https://idp.example.com/saml");
        assert_eq!(cfg["singleSignOnServiceUrl"], "https://idp.example.com/sso");
    }
}

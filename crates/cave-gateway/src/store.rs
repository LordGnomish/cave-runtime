// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! In-memory store for all gateway entities.
//! Thread-safe via DashMap. Supports lookup by id, name, and tag.

use crate::models::*;
use dashmap::DashMap;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, Default)]
pub struct GatewayStore {
    pub services: DashMap<Uuid, Service>,
    pub service_names: DashMap<String, Uuid>,

    pub routes: DashMap<Uuid, Route>,
    pub route_names: DashMap<String, Uuid>,

    pub upstreams: DashMap<Uuid, Upstream>,
    pub upstream_names: DashMap<String, Uuid>,

    pub targets: DashMap<Uuid, Target>, // target_id → Target

    pub consumers: DashMap<Uuid, Consumer>,
    pub consumer_names: DashMap<String, Uuid>, // username → id
    pub consumer_custom: DashMap<String, Uuid>, // custom_id → id

    pub plugins: DashMap<Uuid, Plugin>,

    pub certificates: DashMap<Uuid, Certificate>,
    pub snis: DashMap<Uuid, Sni>,
    pub sni_names: DashMap<String, Uuid>,

    // Consumer credentials
    pub key_auth: DashMap<Uuid, KeyAuthCredential>, // cred_id → cred
    pub key_auth_idx: DashMap<String, Uuid>,        // key → cred_id
    pub jwt_creds: DashMap<Uuid, JwtCredential>,
    pub jwt_key_idx: DashMap<String, Uuid>, // jwt.key → cred_id
    pub basic_auth: DashMap<Uuid, BasicAuthCredential>,
    pub basic_auth_idx: DashMap<String, Uuid>, // username → cred_id
    pub hmac_auth: DashMap<Uuid, HmacAuthCredential>,
    pub hmac_auth_idx: DashMap<String, Uuid>, // username → cred_id
    pub oauth2_creds: DashMap<Uuid, OAuth2Credential>,
    pub oauth2_client_idx: DashMap<String, Uuid>, // client_id → cred_id
    pub acl_groups: DashMap<Uuid, AclGroup>,

    // OAuth2 token store (access_token → consumer_id + expiry)
    pub oauth2_tokens: DashMap<String, OAuth2Token>,
}

#[derive(Debug, Clone)]
pub struct OAuth2Token {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: u64,
    pub consumer_id: Uuid,
    pub client_id: String,
    pub scope: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

pub type SharedStore = Arc<GatewayStore>;

impl GatewayStore {
    pub fn new() -> SharedStore {
        Arc::new(GatewayStore::default())
    }

    // ── Services ──────────────────────────────────────────────────────────

    pub fn insert_service(&self, svc: Service) {
        if let Some(name) = &svc.name {
            self.service_names.insert(name.clone(), svc.id);
        }
        self.services.insert(svc.id, svc);
    }

    pub fn get_service_by_id_or_name(&self, id_or_name: &str) -> Option<Service> {
        if let Ok(id) = id_or_name.parse::<Uuid>() {
            return self.services.get(&id).map(|e| e.value().clone());
        }
        let id = self.service_names.get(id_or_name)?.value().clone();
        self.services.get(&id).map(|e| e.value().clone())
    }

    pub fn list_services(&self) -> Vec<Service> {
        self.services.iter().map(|e| e.value().clone()).collect()
    }

    pub fn delete_service(&self, id: &Uuid) -> bool {
        if let Some((_, svc)) = self.services.remove(id) {
            if let Some(name) = svc.name {
                self.service_names.remove(&name);
            }
            return true;
        }
        false
    }

    // ── Routes ────────────────────────────────────────────────────────────

    pub fn insert_route(&self, route: Route) {
        if let Some(name) = &route.name {
            self.route_names.insert(name.clone(), route.id);
        }
        self.routes.insert(route.id, route);
    }

    pub fn get_route_by_id_or_name(&self, id_or_name: &str) -> Option<Route> {
        if let Ok(id) = id_or_name.parse::<Uuid>() {
            return self.routes.get(&id).map(|e| e.value().clone());
        }
        let id = self.route_names.get(id_or_name)?.value().clone();
        self.routes.get(&id).map(|e| e.value().clone())
    }

    pub fn list_routes(&self) -> Vec<Route> {
        self.routes.iter().map(|e| e.value().clone()).collect()
    }

    pub fn routes_for_service(&self, service_id: &Uuid) -> Vec<Route> {
        self.routes
            .iter()
            .filter(|e| e.value().service_id.as_ref() == Some(service_id))
            .map(|e| e.value().clone())
            .collect()
    }

    pub fn delete_route(&self, id: &Uuid) -> bool {
        if let Some((_, r)) = self.routes.remove(id) {
            if let Some(name) = r.name {
                self.route_names.remove(&name);
            }
            return true;
        }
        false
    }

    // ── Upstreams ─────────────────────────────────────────────────────────

    pub fn insert_upstream(&self, up: Upstream) {
        self.upstream_names.insert(up.name.clone(), up.id);
        self.upstreams.insert(up.id, up);
    }

    pub fn get_upstream_by_id_or_name(&self, id_or_name: &str) -> Option<Upstream> {
        if let Ok(id) = id_or_name.parse::<Uuid>() {
            return self.upstreams.get(&id).map(|e| e.value().clone());
        }
        let id = self.upstream_names.get(id_or_name)?.value().clone();
        self.upstreams.get(&id).map(|e| e.value().clone())
    }

    pub fn list_upstreams(&self) -> Vec<Upstream> {
        self.upstreams.iter().map(|e| e.value().clone()).collect()
    }

    pub fn delete_upstream(&self, id: &Uuid) -> bool {
        if let Some((_, u)) = self.upstreams.remove(id) {
            self.upstream_names.remove(&u.name);
            return true;
        }
        false
    }

    // ── Targets ───────────────────────────────────────────────────────────

    pub fn insert_target(&self, t: Target) {
        self.targets.insert(t.id, t);
    }

    pub fn targets_for_upstream(&self, upstream_id: &Uuid) -> Vec<Target> {
        self.targets
            .iter()
            .filter(|e| &e.value().upstream_id == upstream_id)
            .map(|e| e.value().clone())
            .collect()
    }

    pub fn delete_target(&self, id: &Uuid) -> bool {
        self.targets.remove(id).is_some()
    }

    // ── Consumers ─────────────────────────────────────────────────────────

    pub fn insert_consumer(&self, c: Consumer) {
        if let Some(name) = &c.username {
            self.consumer_names.insert(name.clone(), c.id);
        }
        if let Some(cid) = &c.custom_id {
            self.consumer_custom.insert(cid.clone(), c.id);
        }
        self.consumers.insert(c.id, c);
    }

    pub fn get_consumer_by_id_or_name(&self, id_or_name: &str) -> Option<Consumer> {
        if let Ok(id) = id_or_name.parse::<Uuid>() {
            return self.consumers.get(&id).map(|e| e.value().clone());
        }
        // try username
        if let Some(id) = self.consumer_names.get(id_or_name) {
            return self.consumers.get(id.value()).map(|e| e.value().clone());
        }
        // try custom_id
        if let Some(id) = self.consumer_custom.get(id_or_name) {
            return self.consumers.get(id.value()).map(|e| e.value().clone());
        }
        None
    }

    pub fn list_consumers(&self) -> Vec<Consumer> {
        self.consumers.iter().map(|e| e.value().clone()).collect()
    }

    pub fn delete_consumer(&self, id: &Uuid) -> bool {
        if let Some((_, c)) = self.consumers.remove(id) {
            if let Some(u) = c.username {
                self.consumer_names.remove(&u);
            }
            if let Some(cid) = c.custom_id {
                self.consumer_custom.remove(&cid);
            }
            return true;
        }
        false
    }

    // ── Plugins ───────────────────────────────────────────────────────────

    pub fn insert_plugin(&self, p: Plugin) {
        self.plugins.insert(p.id, p);
    }

    pub fn list_plugins(&self) -> Vec<Plugin> {
        self.plugins.iter().map(|e| e.value().clone()).collect()
    }

    pub fn plugins_for_route(&self, route_id: &Uuid) -> Vec<Plugin> {
        self.plugins
            .iter()
            .filter(|e| {
                let p = e.value();
                p.enabled && p.route_id.as_ref() == Some(route_id)
            })
            .map(|e| e.value().clone())
            .collect()
    }

    pub fn plugins_for_service(&self, service_id: &Uuid) -> Vec<Plugin> {
        self.plugins
            .iter()
            .filter(|e| {
                let p = e.value();
                p.enabled && p.service_id.as_ref() == Some(service_id)
            })
            .map(|e| e.value().clone())
            .collect()
    }

    pub fn global_plugins(&self) -> Vec<Plugin> {
        self.plugins
            .iter()
            .filter(|e| {
                let p = e.value();
                p.enabled
                    && p.service_id.is_none()
                    && p.route_id.is_none()
                    && p.consumer_id.is_none()
            })
            .map(|e| e.value().clone())
            .collect()
    }

    pub fn delete_plugin(&self, id: &Uuid) -> bool {
        self.plugins.remove(id).is_some()
    }

    // ── Certificates ──────────────────────────────────────────────────────

    pub fn insert_certificate(&self, cert: Certificate) {
        self.certificates.insert(cert.id, cert);
    }

    pub fn list_certificates(&self) -> Vec<Certificate> {
        self.certificates
            .iter()
            .map(|e| e.value().clone())
            .collect()
    }

    pub fn delete_certificate(&self, id: &Uuid) -> bool {
        self.certificates.remove(id).is_some()
    }

    // ── SNIs ──────────────────────────────────────────────────────────────

    pub fn insert_sni(&self, sni: Sni) {
        self.sni_names.insert(sni.name.clone(), sni.id);
        self.snis.insert(sni.id, sni);
    }

    pub fn get_sni_by_name(&self, name: &str) -> Option<Sni> {
        let id = self.sni_names.get(name)?.value().clone();
        self.snis.get(&id).map(|e| e.value().clone())
    }

    pub fn list_snis(&self) -> Vec<Sni> {
        self.snis.iter().map(|e| e.value().clone()).collect()
    }

    pub fn delete_sni(&self, id: &Uuid) -> bool {
        if let Some((_, s)) = self.snis.remove(id) {
            self.sni_names.remove(&s.name);
            return true;
        }
        false
    }
}

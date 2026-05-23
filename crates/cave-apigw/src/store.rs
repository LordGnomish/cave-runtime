// SPDX-License-Identifier: AGPL-3.0-or-later
//! In-memory CRUD store keyed by UUID + name. Mirrors Kong's DB-less mode.

use crate::error::{AGwError, AGwResult};
use crate::models::{Consumer, GwConfig, Plugin, Route, Service, Upstream};
use std::collections::HashMap;
use std::sync::RwLock;
use uuid::Uuid;

#[derive(Default)]
pub struct GwStore { inner: RwLock<Inner> }

#[derive(Default)]
struct Inner {
    config: GwConfig,
    routes: HashMap<Uuid, Route>, routes_by_name: HashMap<String, Uuid>,
    services: HashMap<Uuid, Service>, services_by_name: HashMap<String, Uuid>,
    upstreams: HashMap<Uuid, Upstream>, upstreams_by_name: HashMap<String, Uuid>,
    consumers: HashMap<Uuid, Consumer>, consumers_by_username: HashMap<String, Uuid>,
    plugins: HashMap<Uuid, Plugin>,
}

impl GwStore {
    pub fn new() -> Self { Self::default() }
    pub fn set_config(&self, cfg: GwConfig) { self.inner.write().unwrap().config = cfg; }
    pub fn config(&self) -> GwConfig { self.inner.read().unwrap().config.clone() }

    pub fn upsert_route(&self, r: Route) -> AGwResult<Uuid> {
        let mut g = self.inner.write().unwrap();
        if let Some(eid) = g.routes_by_name.get(&r.name) {
            if *eid != r.id { return Err(AGwError::Conflict(format!("route name {}", r.name))); }
        }
        let id = r.id; g.routes_by_name.insert(r.name.clone(), id); g.routes.insert(id, r); Ok(id)
    }
    pub fn get_route(&self, id: Uuid) -> AGwResult<Route> {
        self.inner.read().unwrap().routes.get(&id).cloned()
            .ok_or_else(|| AGwError::RouteNotFound(id.to_string()))
    }
    pub fn get_route_by_name(&self, name: &str) -> AGwResult<Route> {
        let g = self.inner.read().unwrap();
        let id = g.routes_by_name.get(name).copied().ok_or_else(|| AGwError::RouteNotFound(name.into()))?;
        g.routes.get(&id).cloned().ok_or_else(|| AGwError::RouteNotFound(name.into()))
    }
    pub fn delete_route(&self, id: Uuid) -> AGwResult<()> {
        let mut g = self.inner.write().unwrap();
        let r = g.routes.remove(&id).ok_or_else(|| AGwError::RouteNotFound(id.to_string()))?;
        g.routes_by_name.remove(&r.name); Ok(())
    }
    pub fn list_routes(&self) -> Vec<Route> { self.inner.read().unwrap().routes.values().cloned().collect() }

    pub fn upsert_service(&self, s: Service) -> AGwResult<Uuid> {
        let mut g = self.inner.write().unwrap();
        if let Some(eid) = g.services_by_name.get(&s.name) {
            if *eid != s.id { return Err(AGwError::Conflict(format!("service name {}", s.name))); }
        }
        let id = s.id; g.services_by_name.insert(s.name.clone(), id); g.services.insert(id, s); Ok(id)
    }
    pub fn get_service(&self, id: Uuid) -> AGwResult<Service> {
        self.inner.read().unwrap().services.get(&id).cloned()
            .ok_or_else(|| AGwError::ServiceNotFound(id.to_string()))
    }
    pub fn get_service_by_name(&self, name: &str) -> AGwResult<Service> {
        let g = self.inner.read().unwrap();
        let id = g.services_by_name.get(name).copied().ok_or_else(|| AGwError::ServiceNotFound(name.into()))?;
        g.services.get(&id).cloned().ok_or_else(|| AGwError::ServiceNotFound(name.into()))
    }
    pub fn delete_service(&self, id: Uuid) -> AGwResult<()> {
        let mut g = self.inner.write().unwrap();
        let s = g.services.remove(&id).ok_or_else(|| AGwError::ServiceNotFound(id.to_string()))?;
        g.services_by_name.remove(&s.name); Ok(())
    }
    pub fn list_services(&self) -> Vec<Service> { self.inner.read().unwrap().services.values().cloned().collect() }

    pub fn upsert_upstream(&self, u: Upstream) -> AGwResult<Uuid> {
        let mut g = self.inner.write().unwrap();
        if let Some(eid) = g.upstreams_by_name.get(&u.name) {
            if *eid != u.id { return Err(AGwError::Conflict(format!("upstream name {}", u.name))); }
        }
        let id = u.id; g.upstreams_by_name.insert(u.name.clone(), id); g.upstreams.insert(id, u); Ok(id)
    }
    pub fn get_upstream(&self, id: Uuid) -> AGwResult<Upstream> {
        self.inner.read().unwrap().upstreams.get(&id).cloned()
            .ok_or_else(|| AGwError::UpstreamNotFound(id.to_string()))
    }
    pub fn get_upstream_by_name(&self, name: &str) -> AGwResult<Upstream> {
        let g = self.inner.read().unwrap();
        let id = g.upstreams_by_name.get(name).copied().ok_or_else(|| AGwError::UpstreamNotFound(name.into()))?;
        g.upstreams.get(&id).cloned().ok_or_else(|| AGwError::UpstreamNotFound(name.into()))
    }
    pub fn delete_upstream(&self, id: Uuid) -> AGwResult<()> {
        let mut g = self.inner.write().unwrap();
        let u = g.upstreams.remove(&id).ok_or_else(|| AGwError::UpstreamNotFound(id.to_string()))?;
        g.upstreams_by_name.remove(&u.name); Ok(())
    }
    pub fn list_upstreams(&self) -> Vec<Upstream> { self.inner.read().unwrap().upstreams.values().cloned().collect() }

    pub fn upsert_consumer(&self, c: Consumer) -> AGwResult<Uuid> {
        let mut g = self.inner.write().unwrap();
        if let Some(eid) = g.consumers_by_username.get(&c.username) {
            if *eid != c.id { return Err(AGwError::Conflict(format!("consumer username {}", c.username))); }
        }
        let id = c.id; g.consumers_by_username.insert(c.username.clone(), id); g.consumers.insert(id, c); Ok(id)
    }
    pub fn get_consumer(&self, id: Uuid) -> AGwResult<Consumer> {
        self.inner.read().unwrap().consumers.get(&id).cloned()
            .ok_or_else(|| AGwError::ConsumerNotFound(id.to_string()))
    }
    pub fn list_consumers(&self) -> Vec<Consumer> { self.inner.read().unwrap().consumers.values().cloned().collect() }

    pub fn upsert_plugin(&self, p: Plugin) -> AGwResult<Uuid> {
        let id = p.id; self.inner.write().unwrap().plugins.insert(id, p); Ok(id)
    }
    pub fn get_plugin(&self, id: Uuid) -> AGwResult<Plugin> {
        self.inner.read().unwrap().plugins.get(&id).cloned()
            .ok_or_else(|| AGwError::PluginNotFound(id.to_string()))
    }
    pub fn delete_plugin(&self, id: Uuid) -> AGwResult<()> {
        self.inner.write().unwrap().plugins.remove(&id).ok_or_else(|| AGwError::PluginNotFound(id.to_string()))?;
        Ok(())
    }
    pub fn list_plugins(&self) -> Vec<Plugin> { self.inner.read().unwrap().plugins.values().cloned().collect() }

    /// Kong plugin precedence: (route+service+consumer) > (route+service) > (route+consumer)
    /// > (service+consumer) > (route) > (service) > (consumer) > global.
    pub fn effective_plugins(&self, route: Option<Uuid>, service: Option<Uuid>, consumer: Option<Uuid>) -> Vec<Plugin> {
        let g = self.inner.read().unwrap();
        let mut out: Vec<Plugin> = Vec::new();
        let mut seen: HashMap<crate::models::PluginKind, u8> = HashMap::new();
        let scopes: [(Option<Uuid>, Option<Uuid>, Option<Uuid>, u8); 8] = [
            (route, service, consumer, 0), (route, service, None, 1),
            (route, None, consumer, 2), (None, service, consumer, 3),
            (route, None, None, 4), (None, service, None, 5),
            (None, None, consumer, 6), (None, None, None, 7),
        ];
        for (r, s, c, prio) in scopes {
            for p in g.plugins.values() {
                if !p.enabled { continue; }
                if p.route_id == r && p.service_id == s && p.consumer_id == c {
                    let existing = seen.get(&p.kind).copied().unwrap_or(u8::MAX);
                    if prio < existing {
                        seen.insert(p.kind, prio);
                        out.retain(|q| q.kind != p.kind);
                        out.push(p.clone());
                    }
                }
            }
        }
        out
    }

    pub fn snapshot_counts(&self) -> StoreCounts {
        let g = self.inner.read().unwrap();
        StoreCounts { routes: g.routes.len(), services: g.services.len(),
            upstreams: g.upstreams.len(), consumers: g.consumers.len(),
            plugins: g.plugins.len() }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoreCounts { pub routes: usize, pub services: usize, pub upstreams: usize, pub consumers: usize, pub plugins: usize }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Plugin, PluginKind, Route, Service, Upstream};
    #[test] fn route_lifecycle() {
        let s = GwStore::new();
        let id = s.upsert_route(Route::new("r1")).unwrap();
        assert_eq!(s.get_route(id).unwrap().name, "r1");
        assert_eq!(s.get_route_by_name("r1").unwrap().id, id);
        s.delete_route(id).unwrap();
        assert!(s.get_route(id).is_err());
    }
    #[test] fn duplicate_route_name() {
        let s = GwStore::new();
        s.upsert_route(Route::new("r1")).unwrap();
        assert!(matches!(s.upsert_route(Route::new("r1")), Err(AGwError::Conflict(_))));
    }
    #[test] fn service_lifecycle() {
        let s = GwStore::new();
        let id = s.upsert_service(Service::new("svc", "h", 80)).unwrap();
        assert_eq!(s.get_service(id).unwrap().host, "h");
        s.delete_service(id).unwrap();
        assert!(s.get_service(id).is_err());
    }
    #[test] fn upstream_lifecycle() {
        let s = GwStore::new();
        let id = s.upsert_upstream(Upstream::new("api")).unwrap();
        assert_eq!(s.get_upstream(id).unwrap().name, "api");
        s.delete_upstream(id).unwrap();
        assert!(s.get_upstream(id).is_err());
    }
    #[test] fn effective_plugins_route_overrides_global() {
        let s = GwStore::new();
        let rid = s.upsert_route(Route::new("r")).unwrap();
        s.upsert_plugin(Plugin::new("g", PluginKind::KeyAuth)).unwrap();
        s.upsert_plugin(Plugin { route_id: Some(rid), ..Plugin::new("r-kf", PluginKind::KeyAuth) }).unwrap();
        let eff = s.effective_plugins(Some(rid), None, None);
        assert_eq!(eff.len(), 1);
        assert_eq!(eff[0].name, "r-kf");
    }
    #[test] fn snapshot_counts() {
        let s = GwStore::new();
        s.upsert_route(Route::new("r")).unwrap();
        s.upsert_service(Service::new("svc", "h", 80)).unwrap();
        s.upsert_upstream(Upstream::new("up")).unwrap();
        let c = s.snapshot_counts();
        assert_eq!(c.routes, 1); assert_eq!(c.services, 1); assert_eq!(c.upstreams, 1);
    }
}

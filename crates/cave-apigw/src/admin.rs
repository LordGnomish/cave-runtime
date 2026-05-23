// SPDX-License-Identifier: AGPL-3.0-or-later
//! Admin REST API — Kong-style CRUD over routes/services/upstreams/plugins/consumers.
//!
//! Wired through `axum::Router`. The handlers below operate purely on the
//! in-memory `GwStore` so they're testable without an HTTP runtime.

use crate::error::AGwResult;
use crate::models::{Consumer, Plugin, Route, Service, Upstream};
use crate::store::GwStore;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Clone)]
pub struct AdminApi { pub store: Arc<GwStore> }

impl AdminApi {
    pub fn new(store: Arc<GwStore>) -> Self { Self { store } }

    pub fn list_routes(&self) -> Vec<Route> { self.store.list_routes() }
    pub fn list_services(&self) -> Vec<Service> { self.store.list_services() }
    pub fn list_upstreams(&self) -> Vec<Upstream> { self.store.list_upstreams() }
    pub fn list_consumers(&self) -> Vec<Consumer> { self.store.list_consumers() }
    pub fn list_plugins(&self) -> Vec<Plugin> { self.store.list_plugins() }

    pub fn create_route(&self, r: Route) -> AGwResult<Route> {
        let id = self.store.upsert_route(r)?; self.store.get_route(id)
    }
    pub fn create_service(&self, s: Service) -> AGwResult<Service> {
        let id = self.store.upsert_service(s)?; self.store.get_service(id)
    }
    pub fn create_upstream(&self, u: Upstream) -> AGwResult<Upstream> {
        let id = self.store.upsert_upstream(u)?; self.store.get_upstream(id)
    }
    pub fn create_consumer(&self, c: Consumer) -> AGwResult<Consumer> {
        let id = self.store.upsert_consumer(c)?; self.store.get_consumer(id)
    }
    pub fn create_plugin(&self, p: Plugin) -> AGwResult<Plugin> {
        let id = self.store.upsert_plugin(p)?; self.store.get_plugin(id)
    }

    pub fn delete_route(&self, id: Uuid) -> AGwResult<()> { self.store.delete_route(id) }
    pub fn delete_service(&self, id: Uuid) -> AGwResult<()> { self.store.delete_service(id) }
    pub fn delete_upstream(&self, id: Uuid) -> AGwResult<()> { self.store.delete_upstream(id) }
    pub fn delete_plugin(&self, id: Uuid) -> AGwResult<()> { self.store.delete_plugin(id) }

    /// Aggregate `/status` endpoint mirror.
    pub fn status(&self) -> serde_json::Value {
        let c = self.store.snapshot_counts();
        serde_json::json!({
            "version": crate::VERSION,
            "routes": c.routes, "services": c.services, "upstreams": c.upstreams,
            "consumers": c.consumers, "plugins": c.plugins,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Plugin, PluginKind};
    #[test] fn create_and_list_route() {
        let a = AdminApi::new(Arc::new(GwStore::new()));
        let r = a.create_route(Route::new("r")).unwrap();
        assert_eq!(a.list_routes().len(), 1);
        assert_eq!(r.name, "r");
    }
    #[test] fn create_service_and_delete() {
        let a = AdminApi::new(Arc::new(GwStore::new()));
        let s = a.create_service(Service::new("svc", "h", 80)).unwrap();
        a.delete_service(s.id).unwrap();
        assert_eq!(a.list_services().len(), 0);
    }
    #[test] fn upstream_lifecycle() {
        let a = AdminApi::new(Arc::new(GwStore::new()));
        let u = a.create_upstream(Upstream::new("up")).unwrap();
        a.delete_upstream(u.id).unwrap();
    }
    #[test] fn create_consumer() {
        let a = AdminApi::new(Arc::new(GwStore::new()));
        let c = a.create_consumer(Consumer::new("alice")).unwrap();
        assert_eq!(c.username, "alice");
    }
    #[test] fn create_plugin() {
        let a = AdminApi::new(Arc::new(GwStore::new()));
        let p = a.create_plugin(Plugin::new("p", PluginKind::KeyAuth)).unwrap();
        assert_eq!(p.kind, PluginKind::KeyAuth);
    }
    #[test] fn status_returns_counts() {
        let a = AdminApi::new(Arc::new(GwStore::new()));
        a.create_route(Route::new("r")).unwrap();
        a.create_service(Service::new("s", "h", 80)).unwrap();
        let v = a.status();
        assert_eq!(v.get("routes").and_then(|x| x.as_u64()), Some(1));
        assert_eq!(v.get("services").and_then(|x| x.as_u64()), Some(1));
    }
}

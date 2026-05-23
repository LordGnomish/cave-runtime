// SPDX-License-Identifier: AGPL-3.0-or-later
//! cavectl subcommand handler — `cave gw {route,service,plugin,consumer,upstream}`.
//!
//! Parsed args + execution against a passed-in `GwStore`. The cave-cli binary
//! wires this in `main.rs`.

use crate::error::AGwResult;
use crate::models::{Consumer, Plugin, PluginKind, Route, Service, Upstream};
use crate::store::GwStore;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub enum GwCommand {
    RouteList,
    RouteCreate { name: String, paths: Vec<String>, service: Option<String> },
    RouteDelete { name: String },
    ServiceList,
    ServiceCreate { name: String, host: String, port: u16 },
    ServiceDelete { name: String },
    UpstreamList,
    UpstreamCreate { name: String, algorithm: String },
    UpstreamDelete { name: String },
    ConsumerList,
    ConsumerCreate { username: String },
    PluginList,
    PluginCreate { name: String, kind: String },
    Status,
}

pub fn execute(store: &Arc<GwStore>, cmd: GwCommand) -> AGwResult<String> {
    match cmd {
        GwCommand::RouteList => Ok(serde_json::to_string_pretty(&store.list_routes())?),
        GwCommand::RouteCreate { name, paths, service } => {
            let mut r = Route::new(&name); r.paths = paths;
            if let Some(svc) = service {
                let s = store.get_service_by_name(&svc)?;
                r.service_id = Some(s.id);
            }
            store.upsert_route(r)?;
            Ok(format!("route '{name}' created"))
        }
        GwCommand::RouteDelete { name } => {
            let r = store.get_route_by_name(&name)?;
            store.delete_route(r.id)?;
            Ok(format!("route '{name}' deleted"))
        }
        GwCommand::ServiceList => Ok(serde_json::to_string_pretty(&store.list_services())?),
        GwCommand::ServiceCreate { name, host, port } => {
            store.upsert_service(Service::new(&name, &host, port))?;
            Ok(format!("service '{name}' created"))
        }
        GwCommand::ServiceDelete { name } => {
            let s = store.get_service_by_name(&name)?;
            store.delete_service(s.id)?;
            Ok(format!("service '{name}' deleted"))
        }
        GwCommand::UpstreamList => Ok(serde_json::to_string_pretty(&store.list_upstreams())?),
        GwCommand::UpstreamCreate { name, algorithm } => {
            let mut u = Upstream::new(&name);
            u.algorithm = match algorithm.as_str() {
                "round-robin" => crate::models::UpstreamAlgorithm::RoundRobin,
                "least-connections" => crate::models::UpstreamAlgorithm::LeastConnections,
                "consistent-hashing" => crate::models::UpstreamAlgorithm::ConsistentHashing,
                "ewma" => crate::models::UpstreamAlgorithm::Ewma,
                "random" => crate::models::UpstreamAlgorithm::Random,
                _ => crate::models::UpstreamAlgorithm::RoundRobin,
            };
            store.upsert_upstream(u)?;
            Ok(format!("upstream '{name}' created"))
        }
        GwCommand::UpstreamDelete { name } => {
            let u = store.get_upstream_by_name(&name)?;
            store.delete_upstream(u.id)?;
            Ok(format!("upstream '{name}' deleted"))
        }
        GwCommand::ConsumerList => Ok(serde_json::to_string_pretty(&store.list_consumers())?),
        GwCommand::ConsumerCreate { username } => {
            store.upsert_consumer(Consumer::new(&username))?;
            Ok(format!("consumer '{username}' created"))
        }
        GwCommand::PluginList => Ok(serde_json::to_string_pretty(&store.list_plugins())?),
        GwCommand::PluginCreate { name, kind } => {
            let k = parse_kind(&kind)?;
            store.upsert_plugin(Plugin::new(&name, k))?;
            Ok(format!("plugin '{name}' created"))
        }
        GwCommand::Status => {
            let c = store.snapshot_counts();
            Ok(format!("routes={} services={} upstreams={} consumers={} plugins={}",
                c.routes, c.services, c.upstreams, c.consumers, c.plugins))
        }
    }
}

fn parse_kind(s: &str) -> AGwResult<PluginKind> {
    match s {
        "key-auth" => Ok(PluginKind::KeyAuth),
        "jwt" => Ok(PluginKind::Jwt),
        "oauth2" => Ok(PluginKind::Oauth2),
        "mtls" => Ok(PluginKind::Mtls),
        "ldap" => Ok(PluginKind::Ldap),
        "rate-limiting" => Ok(PluginKind::RateLimiting),
        "proxy-cache" => Ok(PluginKind::ProxyCache),
        "request-transformer" => Ok(PluginKind::RequestTransformer),
        "response-transformer" => Ok(PluginKind::ResponseTransformer),
        "cors" => Ok(PluginKind::Cors),
        "bot-detection" => Ok(PluginKind::BotDetection),
        "ip-restriction" => Ok(PluginKind::IpRestriction),
        "circuit-breaker" => Ok(PluginKind::CircuitBreaker),
        "retry" => Ok(PluginKind::Retry),
        "request-termination" => Ok(PluginKind::RequestTermination),
        _ => Err(crate::error::AGwError::BadRequest(format!("unknown plugin kind {s}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn status_format() {
        let s = Arc::new(GwStore::new());
        let out = execute(&s, GwCommand::Status).unwrap();
        assert!(out.contains("routes=0"));
    }
    #[test] fn service_create_list_delete() {
        let s = Arc::new(GwStore::new());
        execute(&s, GwCommand::ServiceCreate { name: "svc".into(), host: "h".into(), port: 80 }).unwrap();
        assert_eq!(s.list_services().len(), 1);
        execute(&s, GwCommand::ServiceDelete { name: "svc".into() }).unwrap();
        assert_eq!(s.list_services().len(), 0);
    }
    #[test] fn route_create_with_service() {
        let s = Arc::new(GwStore::new());
        execute(&s, GwCommand::ServiceCreate { name: "svc".into(), host: "h".into(), port: 80 }).unwrap();
        execute(&s, GwCommand::RouteCreate { name: "r".into(), paths: vec!["/api".into()], service: Some("svc".into()) }).unwrap();
        let r = s.get_route_by_name("r").unwrap();
        assert!(r.service_id.is_some());
    }
    #[test] fn upstream_algo_parse() {
        let s = Arc::new(GwStore::new());
        execute(&s, GwCommand::UpstreamCreate { name: "u".into(), algorithm: "ewma".into() }).unwrap();
        assert_eq!(s.get_upstream_by_name("u").unwrap().algorithm, crate::models::UpstreamAlgorithm::Ewma);
    }
    #[test] fn plugin_create() {
        let s = Arc::new(GwStore::new());
        execute(&s, GwCommand::PluginCreate { name: "p".into(), kind: "key-auth".into() }).unwrap();
        assert_eq!(s.list_plugins().len(), 1);
    }
    #[test] fn plugin_unknown_kind() {
        let s = Arc::new(GwStore::new());
        assert!(execute(&s, GwCommand::PluginCreate { name: "p".into(), kind: "nope".into() }).is_err());
    }
    #[test] fn consumer_create() {
        let s = Arc::new(GwStore::new());
        execute(&s, GwCommand::ConsumerCreate { username: "alice".into() }).unwrap();
        assert_eq!(s.list_consumers().len(), 1);
    }
}

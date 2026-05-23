// SPDX-License-Identifier: AGPL-3.0-or-later
//! Smoke end-to-end — exercises store, router, plugins, proxy, admin, declarative,
//! cli, observability, and CRDs in a single test pass.

use cave_apigw::*;
use std::sync::Arc;

fn make_store() -> Arc<store::GwStore> {
    let s = Arc::new(store::GwStore::new());
    let svc_id = s.upsert_service(Service::new("svc", "127.0.0.1", 8080)).unwrap();
    let mut r = Route::new("default");
    r.paths = vec!["/api".into()];
    r.service_id = Some(svc_id);
    s.upsert_route(r).unwrap();
    s
}

#[test]
fn smoke_route_matches() {
    let s = make_store();
    let router = router::Router::new(s);
    let ctx = router::RequestCtx {
        method: "GET".into(), host: "h".into(), path: "/api/users".into(),
        headers: vec![], sni: None, protocol: Protocol::Http,
        source_ip: Some("1.2.3.4".into()), destination_port: None,
    };
    let m = router.r#match(&ctx).unwrap();
    assert_eq!(m.route.name, "default");
    assert_eq!(m.rewritten_path, "/users");
}

#[test]
fn smoke_proxy_forwards_static_response() {
    let store = make_store();
    let lb = Arc::new(lb::LbState::new());
    let client = Arc::new(proxy::StaticUpstream { response: proxy::GwResponse::new(200).body(b"hi".to_vec()) });
    let p = proxy::Proxy::new(store.clone(), lb, client);
    let route = store.get_route_by_name("default").unwrap();
    let svc = store.get_service(route.service_id.unwrap()).unwrap();
    let out = p.handle(&route, Some(&svc), &[proxy::service_as_target(&svc)],
        proxy::GwRequest::new("GET", "/api/x", "h"), "/x".into()).unwrap();
    assert_eq!(out.response.status, 200);
    assert_eq!(out.response.body, b"hi");
}

#[test]
fn smoke_plugin_chain_runs() {
    let store = make_store();
    let route = store.get_route_by_name("default").unwrap();
    let plugin = Plugin { route_id: Some(route.id), ..Plugin::new("cors", PluginKind::Cors) };
    store.upsert_plugin(plugin).unwrap();
    let lb = Arc::new(lb::LbState::new());
    let client = Arc::new(proxy::StaticUpstream { response: proxy::GwResponse::new(200) });
    let p = proxy::Proxy::new(store.clone(), lb, client);
    let svc = store.get_service(route.service_id.unwrap()).unwrap();
    let out = p.handle(&route, Some(&svc), &[proxy::service_as_target(&svc)],
        proxy::GwRequest::new("GET", "/api/x", "h"), "/x".into()).unwrap();
    assert!(out.plugins_run.iter().any(|n| n == "cors"));
}

#[test]
fn smoke_key_auth_plugin_blocks_missing_key() {
    let store = make_store();
    let route = store.get_route_by_name("default").unwrap();
    store.upsert_plugin(Plugin { route_id: Some(route.id), ..Plugin::new("kf", PluginKind::KeyAuth) }).unwrap();
    let lb = Arc::new(lb::LbState::new());
    let client = Arc::new(proxy::StaticUpstream { response: proxy::GwResponse::new(200) });
    let p = proxy::Proxy::new(store.clone(), lb, client);
    let svc = store.get_service(route.service_id.unwrap()).unwrap();
    let res = p.handle(&route, Some(&svc), &[proxy::service_as_target(&svc)],
        proxy::GwRequest::new("GET", "/api/x", "h"), "/x".into());
    assert!(res.is_err()); // 401
}

#[test]
fn smoke_admin_api() {
    let s = Arc::new(store::GwStore::new());
    let a = admin::AdminApi::new(s.clone());
    a.create_service(Service::new("svc", "h", 80)).unwrap();
    a.create_route(Route::new("r")).unwrap();
    a.create_upstream(Upstream::new("up")).unwrap();
    a.create_consumer(Consumer::new("alice")).unwrap();
    a.create_plugin(Plugin::new("p", PluginKind::Cors)).unwrap();
    let v = a.status();
    assert_eq!(v.get("services").and_then(|x| x.as_u64()), Some(1));
    assert_eq!(v.get("routes").and_then(|x| x.as_u64()), Some(1));
    assert_eq!(v.get("plugins").and_then(|x| x.as_u64()), Some(1));
}

#[test]
fn smoke_declarative_round_trip() {
    let s = make_store();
    let yaml = declarative::export_yaml(&s).unwrap();
    let s2 = Arc::new(store::GwStore::new());
    declarative::import_yaml(&s2, &yaml).unwrap();
    assert_eq!(s2.list_routes().len(), 1);
    assert_eq!(s2.list_services().len(), 1);
}

#[test]
fn smoke_cli_status_format() {
    let s = Arc::new(store::GwStore::new());
    let out = cli::execute(&s, cli::GwCommand::Status).unwrap();
    assert!(out.contains("routes=0"));
}

#[test]
fn smoke_cli_creates_then_lists() {
    let s = Arc::new(store::GwStore::new());
    cli::execute(&s, cli::GwCommand::ServiceCreate { name: "svc".into(), host: "h".into(), port: 80 }).unwrap();
    cli::execute(&s, cli::GwCommand::RouteCreate { name: "r".into(), paths: vec!["/api".into()], service: Some("svc".into()) }).unwrap();
    cli::execute(&s, cli::GwCommand::PluginCreate { name: "kf".into(), kind: "key-auth".into() }).unwrap();
    let out = cli::execute(&s, cli::GwCommand::Status).unwrap();
    assert!(out.contains("routes=1"));
    assert!(out.contains("services=1"));
    assert!(out.contains("plugins=1"));
}

#[test]
fn smoke_metrics_observe() {
    let m = metrics::Metrics::new();
    m.inc_requests();
    m.observe_status(200);
    m.observe_latency_ms(42);
    let s = m.snapshot();
    assert_eq!(s.requests_total, 1);
    assert_eq!(s.upstream_2xx_total, 1);
    assert_eq!(s.latency_sum_ms, 42);
}

#[test]
fn smoke_tracing_propagation() {
    let root = tracing_otel::SpanCtx::root();
    let child = root.child();
    assert_eq!(child.trace_id, root.trace_id);
    let tp = root.traceparent_header();
    let parsed = tracing_otel::SpanCtx::from_traceparent(&tp).unwrap();
    assert_eq!(parsed.trace_id, root.trace_id);
}

#[test]
fn smoke_crd_gateway_validate() {
    let mut g = crd::Gateway::new("gw", "cave-class");
    g.listeners.push(crd::GatewayListener { name: "http".into(), port: 80, protocol: "HTTP".into(), hostname: None, tls: None });
    g.validate().unwrap();
}

#[test]
fn smoke_crd_httproute_validate() {
    let mut r = crd::HttpRoute::new("r");
    r.rules.push(crd::HttpRouteRule {
        matches: vec![], backends: vec![crd::HttpBackend { name: "svc".into(), port: 80, weight: 1 }], filters: vec![],
    });
    r.validate().unwrap();
}

#[test]
fn smoke_websocket_upgrade_validate() {
    let mut h: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    h.insert("upgrade".into(), "websocket".into());
    h.insert("connection".into(), "Upgrade".into());
    h.insert("sec-websocket-key".into(), "abc".into());
    let k = websocket::validate_upgrade(&h).unwrap();
    let accept = websocket::derive_accept(k);
    assert!(!accept.is_empty());
}

#[test]
fn smoke_grpc_transcode_lookup() {
    let mut t = grpc::Transcoder::new();
    t.add(grpc::TranscodingRule {
        http_method: "GET".into(), http_path: "/v1/users/{id}".into(),
        grpc: grpc::GrpcMethod { package: "u.v1".into(), service: "Svc".into(), method: "Get".into() },
        body_field: None,
    }).unwrap();
    let (m, _) = t.lookup("GET", "/v1/users/42").unwrap();
    assert_eq!(m.method, "Get");
}

#[test]
fn smoke_pqc_hybrid_codepoint() {
    let p = pqc::KemPolicy::enable_hybrid();
    assert_eq!(p.group_codepoint(), Some(0x11ec));
    assert!(p.is_quantum_resistant());
}

#[test]
fn smoke_acme_hook_issues_cert() {
    use acme_hook::AcmeProvider;
    let p = acme_hook::StubAcme;
    let e = p.issue("api.example").unwrap();
    assert_eq!(e.host, "api.example");
}

#[test]
fn smoke_tls_registry_resolves() {
    let r = tls::CertRegistry::new();
    r.insert(tls::CertEntry {
        host: "api.example".into(), leaf_pem: "L".into(), chain_pem: "C".into(), key_pem: "K".into(),
        not_before: chrono::Utc::now(), not_after: chrono::Utc::now() + chrono::Duration::days(30),
        fingerprint_sha256: "abc".into(),
    });
    assert_eq!(r.resolve("api.example").unwrap().host, "api.example");
}

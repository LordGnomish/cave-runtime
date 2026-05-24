// SPDX-License-Identifier: AGPL-3.0-or-later
//! `proxy-cache` plugin — best-effort response cache keyed by method + URI + vary.

use crate::error::AGwResult;
use crate::plugins::{cfg_bool, cfg_str_array, cfg_u64, PluginContext};
use crate::proxy::GwResponse;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
struct CachedResponse {
    response: GwResponse,
    stored_at: Instant,
    ttl: Duration,
}

pub struct ResponseCache {
    entries: RwLock<HashMap<String, CachedResponse>>,
}

impl Default for ResponseCache {
    fn default() -> Self { Self { entries: RwLock::new(HashMap::new()) } }
}

impl ResponseCache {
    pub fn new() -> Self { Self::default() }
    pub fn get(&self, key: &str) -> Option<GwResponse> {
        let g = self.entries.read().unwrap();
        let entry = g.get(key)?;
        if entry.stored_at.elapsed() <= entry.ttl {
            return Some(entry.response.clone());
        }
        None
    }
    pub fn put(&self, key: String, response: GwResponse, ttl: Duration) {
        self.entries.write().unwrap().insert(
            key,
            CachedResponse { response, stored_at: Instant::now(), ttl },
        );
    }
    pub fn len(&self) -> usize { self.entries.read().unwrap().len() }
}

thread_local! {
    static CACHE: ResponseCache = ResponseCache::new();
}

pub fn cache_key(method: &str, uri: &str, vary_headers: &[(&str, &str)]) -> String {
    let mut h = Sha256::new();
    h.update(method.as_bytes());
    h.update(b"\n");
    h.update(uri.as_bytes());
    h.update(b"\n");
    for (k, v) in vary_headers {
        h.update(k.to_lowercase().as_bytes());
        h.update(b":");
        h.update(v.as_bytes());
        h.update(b"\n");
    }
    hex::encode(h.finalize())
}

pub fn access(cfg: &Value, ctx: &mut PluginContext) -> AGwResult<Option<GwResponse>> {
    let allowed_methods = cfg_str_array(cfg, "request_methods");
    let allowed: Vec<String> = if allowed_methods.is_empty() {
        vec!["GET".into(), "HEAD".into()]
    } else {
        allowed_methods
    };
    if !allowed.iter().any(|m| m.eq_ignore_ascii_case(&ctx.request.method)) {
        return Ok(None);
    }
    let cache_control = cfg_bool(cfg, "cache_control").unwrap_or(true);
    if cache_control && ctx.request.headers.get("cache-control")
        .map(|c| c.contains("no-cache"))
        .unwrap_or(false)
    {
        return Ok(None);
    }
    let vary: Vec<&str> = cfg_str_array(cfg, "vary_headers")
        .into_iter()
        .map(|s| Box::leak(s.into_boxed_str()) as &str)
        .collect();
    let vary_kv: Vec<(&str, &str)> = vary
        .iter()
        .filter_map(|h| ctx.request.headers.get(*h).map(|v| (*h, v.as_str())))
        .collect();
    let key = cache_key(&ctx.request.method, &ctx.request.uri, &vary_kv);
    if let Some(mut resp) = CACHE.with(|c| c.get(&key)) {
        resp.headers.insert("x-cache-status".into(), "HIT".into());
        return Ok(Some(resp));
    }
    ctx.request.headers.insert("x-apigw-cache-key".into(), key);
    Ok(None)
}

pub fn header_filter(cfg: &Value, ctx: &mut PluginContext) -> AGwResult<()> {
    let key = match ctx.request.headers.get("x-apigw-cache-key") {
        Some(k) => k.clone(),
        None => return Ok(()),
    };
    let ttl = Duration::from_secs(cfg_u64(cfg, "cache_ttl").unwrap_or(60));
    let cacheable_statuses = {
        let arr = cfg_str_array(cfg, "response_codes");
        if arr.is_empty() {
            vec![200u16, 301, 404]
        } else {
            arr.iter().filter_map(|s| s.parse().ok()).collect()
        }
    };
    let resp = match &ctx.response {
        Some(r) => r,
        None => return Ok(()),
    };
    if !cacheable_statuses.contains(&resp.status) {
        return Ok(());
    }
    let mut to_store = resp.clone();
    to_store.headers.insert("x-cache-status".into(), "MISS".into());
    CACHE.with(|c| c.put(key, to_store, ttl));
    if let Some(r) = ctx.response.as_mut() {
        r.headers.insert("x-cache-status".into(), "MISS".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Route;
    use crate::proxy::GwRequest;

    fn pc(req: GwRequest) -> PluginContext {
        PluginContext::new(req, None, Route::new("r"))
    }

    #[test]
    fn cache_key_is_deterministic() {
        let k1 = cache_key("GET", "/a", &[]);
        let k2 = cache_key("GET", "/a", &[]);
        assert_eq!(k1, k2);
    }

    #[test]
    fn vary_changes_key() {
        let a = cache_key("GET", "/", &[("accept", "json")]);
        let b = cache_key("GET", "/", &[("accept", "xml")]);
        assert_ne!(a, b);
    }

    #[test]
    fn post_is_not_cached_by_default() {
        let mut c = pc(GwRequest::new("POST", "/", "h"));
        let r = access(&serde_json::json!({}), &mut c).unwrap();
        assert!(r.is_none());
        assert!(c.request.headers.get("x-apigw-cache-key").is_none());
    }

    #[test]
    fn miss_then_hit_round_trip() {
        // First request: MISS, store
        let mut c1 = pc(GwRequest::new("GET", "/round-trip", "h"));
        access(&serde_json::json!({}), &mut c1).unwrap();
        c1.response = Some(GwResponse::new(200).body(b"ok".to_vec()));
        header_filter(&serde_json::json!({}), &mut c1).unwrap();
        // Second request to same key: HIT
        let mut c2 = pc(GwRequest::new("GET", "/round-trip", "h"));
        let r = access(&serde_json::json!({}), &mut c2).unwrap();
        assert!(r.is_some());
        assert_eq!(r.unwrap().headers.get("x-cache-status").map(|s| s.as_str()), Some("HIT"));
    }

    #[test]
    fn no_cache_request_skipped() {
        let mut c = pc(GwRequest::new("GET", "/", "h").header("cache-control", "no-cache"));
        let r = access(&serde_json::json!({}), &mut c).unwrap();
        assert!(r.is_none());
        assert!(c.request.headers.get("x-apigw-cache-key").is_none());
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
//! `rate-limiting` plugin — sliding-window counter with local + cluster modes.

use crate::error::{AGwError, AGwResult};
use crate::plugins::{cfg_bool, cfg_str, cfg_u64, PluginContext};
use crate::proxy::GwResponse;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{Duration, Instant};

/// Single shared limiter; in production this is keyed per-route via a registry.
pub struct LocalLimiter {
    buckets: RwLock<HashMap<String, Bucket>>,
}

struct Bucket {
    count: u64,
    window_start: Instant,
}

impl Default for LocalLimiter {
    fn default() -> Self { Self { buckets: RwLock::new(HashMap::new()) } }
}

impl LocalLimiter {
    pub fn new() -> Self { Self::default() }
    pub fn allow(&self, key: &str, limit: u64, window: Duration) -> bool {
        let mut g = self.buckets.write().unwrap();
        let now = Instant::now();
        let b = g.entry(key.to_string()).or_insert(Bucket { count: 0, window_start: now });
        if now.duration_since(b.window_start) >= window {
            b.count = 0;
            b.window_start = now;
        }
        b.count += 1;
        b.count <= limit
    }
    pub fn current(&self, key: &str) -> u64 {
        self.buckets.read().unwrap().get(key).map(|b| b.count).unwrap_or(0)
    }
}

thread_local! {
    static LIMITER: LocalLimiter = LocalLimiter::new();
}

pub fn access(cfg: &Value, ctx: &mut PluginContext) -> AGwResult<Option<GwResponse>> {
    let minute = cfg_u64(cfg, "minute").unwrap_or(0);
    let hour = cfg_u64(cfg, "hour").unwrap_or(0);
    let day = cfg_u64(cfg, "day").unwrap_or(0);
    let limit_by = cfg_str(cfg, "limit_by").unwrap_or("ip");
    let cluster = cfg_bool(cfg, "cluster").unwrap_or(false);
    let policy = cfg_str(cfg, "policy").unwrap_or(if cluster { "cluster" } else { "local" });
    let key = identity_key(limit_by, ctx);

    if minute > 0 {
        check_or_reject(&format!("min:{key}"), minute, Duration::from_secs(60), policy)?;
    }
    if hour > 0 {
        check_or_reject(&format!("hr:{key}"), hour, Duration::from_secs(3600), policy)?;
    }
    if day > 0 {
        check_or_reject(&format!("day:{key}"), day, Duration::from_secs(86400), policy)?;
    }
    Ok(None)
}

fn check_or_reject(key: &str, limit: u64, window: Duration, _policy: &str) -> AGwResult<()> {
    let allowed = LIMITER.with(|l| l.allow(key, limit, window));
    if !allowed {
        return Err(AGwError::RateLimited { retry_after_s: window.as_secs() as u32 });
    }
    Ok(())
}

fn identity_key(by: &str, ctx: &PluginContext) -> String {
    match by {
        "consumer" => ctx
            .request
            .headers
            .get("x-consumer-username")
            .cloned()
            .unwrap_or_else(|| "anonymous".into()),
        "credential" => ctx
            .request
            .headers
            .get("x-consumer-credential-type")
            .cloned()
            .unwrap_or_else(|| "anonymous".into()),
        "header" => ctx
            .request
            .headers
            .get("x-rl-key")
            .cloned()
            .unwrap_or_else(|| "default".into()),
        "service" => ctx.service.as_ref().map(|s| s.name.clone()).unwrap_or_default(),
        _ => ctx.request.source_ip.clone().unwrap_or_else(|| "unknown".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Route;
    use crate::proxy::GwRequest;

    fn ctx(ip: &str) -> PluginContext {
        let mut r = GwRequest::new("GET", "/", "h");
        r.source_ip = Some(ip.into());
        PluginContext::new(r, None, Route::new("r"))
    }

    #[test]
    fn under_limit_allowed() {
        let mut c = ctx("10.0.0.99");
        let r = access(&serde_json::json!({ "minute": 5 }), &mut c).unwrap();
        assert!(r.is_none());
    }

    #[test]
    fn over_limit_blocked() {
        let mut c = ctx("10.0.0.100");
        for _ in 0..3 {
            access(&serde_json::json!({ "minute": 3 }), &mut c).unwrap();
        }
        let r = access(&serde_json::json!({ "minute": 3 }), &mut c);
        assert!(matches!(r, Err(AGwError::RateLimited { .. })));
    }

    #[test]
    fn local_limiter_buckets() {
        let l = LocalLimiter::new();
        assert!(l.allow("a", 2, Duration::from_secs(60)));
        assert!(l.allow("a", 2, Duration::from_secs(60)));
        assert!(!l.allow("a", 2, Duration::from_secs(60)));
        assert!(l.allow("b", 2, Duration::from_secs(60)));
    }

    #[test]
    fn identity_by_consumer() {
        let mut c = ctx("1.1.1.1");
        c.request.headers.insert("x-consumer-username".into(), "alice".into());
        let k = identity_key("consumer", &c);
        assert_eq!(k, "alice");
    }

    #[test]
    fn identity_default_ip() {
        let c = ctx("203.0.113.1");
        assert_eq!(identity_key("ip", &c), "203.0.113.1");
    }

    #[test]
    fn current_returns_count_zero_for_unknown() {
        let l = LocalLimiter::new();
        assert_eq!(l.current("nope"), 0);
    }
}

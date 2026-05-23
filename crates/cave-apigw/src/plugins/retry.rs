// SPDX-License-Identifier: AGPL-3.0-or-later
//! `retry` plugin — exponential backoff + jitter policy.

use crate::error::AGwResult;
use crate::plugins::{cfg_str_array, cfg_u64, PluginContext};
use crate::proxy::GwResponse;
use serde_json::Value;
use std::time::Duration;

#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
    pub max_attempts: u32, pub base_backoff_ms: u64, pub max_backoff_ms: u64, pub jitter_ms: u64,
}
impl Default for RetryPolicy {
    fn default() -> Self { Self { max_attempts: 3, base_backoff_ms: 50, max_backoff_ms: 2_000, jitter_ms: 25 } }
}
impl RetryPolicy {
    pub fn backoff_for(&self, attempt: u32) -> Duration {
        let raw = self.base_backoff_ms.saturating_mul(1u64 << attempt.min(20));
        Duration::from_millis(raw.min(self.max_backoff_ms))
    }
}

pub fn access(cfg: &Value, ctx: &mut PluginContext) -> AGwResult<Option<GwResponse>> {
    let p = RetryPolicy {
        max_attempts: cfg_u64(cfg, "max_attempts").unwrap_or(3) as u32,
        base_backoff_ms: cfg_u64(cfg, "base_backoff_ms").unwrap_or(50),
        max_backoff_ms: cfg_u64(cfg, "max_backoff_ms").unwrap_or(2_000),
        jitter_ms: cfg_u64(cfg, "jitter_ms").unwrap_or(25),
    };
    let methods = cfg_str_array(cfg, "retry_methods");
    let active = methods.is_empty() || methods.iter().any(|m| m.eq_ignore_ascii_case(&ctx.request.method));
    ctx.request.headers.insert("x-apigw-retry-attempts".into(), p.max_attempts.to_string());
    ctx.request.headers.insert("x-apigw-retry-active".into(), if active { "1".into() } else { "0".into() });
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Route;
    use crate::proxy::GwRequest;
    fn pc(req: GwRequest) -> PluginContext { PluginContext::new(req, None, Route::new("r")) }
    #[test] fn defaults() {
        let p = RetryPolicy::default();
        assert_eq!(p.max_attempts, 3); assert_eq!(p.base_backoff_ms, 50);
    }
    #[test] fn backoff_caps() {
        let p = RetryPolicy { base_backoff_ms: 100, max_backoff_ms: 800, jitter_ms: 0, max_attempts: 5 };
        assert_eq!(p.backoff_for(0).as_millis(), 100);
        assert_eq!(p.backoff_for(1).as_millis(), 200);
        assert_eq!(p.backoff_for(2).as_millis(), 400);
        assert_eq!(p.backoff_for(3).as_millis(), 800);
        assert_eq!(p.backoff_for(4).as_millis(), 800);
    }
    #[test] fn active_when_method_matches() {
        let mut c = pc(GwRequest::new("GET", "/", "h"));
        access(&serde_json::json!({ "retry_methods": ["GET"], "max_attempts": 4 }), &mut c).unwrap();
        assert_eq!(c.request.headers.get("x-apigw-retry-active").map(|s| s.as_str()), Some("1"));
        assert_eq!(c.request.headers.get("x-apigw-retry-attempts").map(|s| s.as_str()), Some("4"));
    }
    #[test] fn inactive_when_excluded() {
        let mut c = pc(GwRequest::new("POST", "/", "h"));
        access(&serde_json::json!({ "retry_methods": ["GET"] }), &mut c).unwrap();
        assert_eq!(c.request.headers.get("x-apigw-retry-active").map(|s| s.as_str()), Some("0"));
    }
    #[test] fn empty_methods_means_all() {
        let mut c = pc(GwRequest::new("DELETE", "/", "h"));
        access(&serde_json::json!({}), &mut c).unwrap();
        assert_eq!(c.request.headers.get("x-apigw-retry-active").map(|s| s.as_str()), Some("1"));
    }
}

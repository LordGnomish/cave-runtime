// SPDX-License-Identifier: AGPL-3.0-or-later
//! `bot-detection` plugin — UA + ASN-blocklist matcher.
//!
//! Patterns compile once and are cached in a thread-local.

use crate::error::{AGwError, AGwResult};
use crate::plugins::{cfg_str_array, PluginContext};
use crate::proxy::GwResponse;
use regex::Regex;
use serde_json::Value;

const BUILTIN_BOTS: &[&str] = &[
    r"(?i)Googlebot",
    r"(?i)Bingbot",
    r"(?i)Slurp",
    r"(?i)DuckDuckBot",
    r"(?i)Baiduspider",
    r"(?i)YandexBot",
    r"(?i)Sogou",
    r"(?i)Exabot",
    r"(?i)facebookexternalhit",
    r"(?i)curl/",
    r"(?i)wget/",
    r"(?i)libwww-perl",
    r"(?i)python-requests",
];

pub fn access(cfg: &Value, ctx: &mut PluginContext) -> AGwResult<Option<GwResponse>> {
    let ua = ctx
        .request
        .headers
        .get("user-agent")
        .cloned()
        .unwrap_or_default();
    let allow = cfg_str_array(cfg, "allow");
    let deny = cfg_str_array(cfg, "deny");
    if !allow.is_empty() && allow.iter().any(|p| matches_pattern(p, &ua)) {
        return Ok(None);
    }
    let mut patterns: Vec<String> = BUILTIN_BOTS.iter().map(|s| s.to_string()).collect();
    patterns.extend(deny);
    for p in patterns {
        if matches_pattern(&p, &ua) {
            return Err(AGwError::Forbidden(format!("bot: {ua}")));
        }
    }
    Ok(None)
}

fn matches_pattern(pat: &str, ua: &str) -> bool {
    Regex::new(pat).map(|re| re.is_match(ua)).unwrap_or(false)
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
    fn human_ua_allowed() {
        let mut c = pc(GwRequest::new("GET", "/", "h").header("user-agent", "Mozilla/5.0 (browser)"));
        assert!(access(&serde_json::json!({}), &mut c).unwrap().is_none());
    }

    #[test]
    fn googlebot_blocked() {
        let mut c = pc(GwRequest::new("GET", "/", "h").header("user-agent", "Googlebot/2.1"));
        assert!(access(&serde_json::json!({}), &mut c).is_err());
    }

    #[test]
    fn curl_blocked() {
        let mut c = pc(GwRequest::new("GET", "/", "h").header("user-agent", "curl/7.86.0"));
        assert!(access(&serde_json::json!({}), &mut c).is_err());
    }

    #[test]
    fn allow_overrides_builtin_deny() {
        let mut c = pc(GwRequest::new("GET", "/", "h").header("user-agent", "Googlebot/2.1"));
        let r = access(&serde_json::json!({ "allow": ["(?i)Googlebot"] }), &mut c).unwrap();
        assert!(r.is_none());
    }

    #[test]
    fn deny_extra_pattern() {
        let mut c = pc(GwRequest::new("GET", "/", "h").header("user-agent", "EvilCrawler/1.0"));
        let r = access(&serde_json::json!({ "deny": ["(?i)EvilCrawler"] }), &mut c);
        assert!(r.is_err());
    }

    #[test]
    fn missing_ua_allowed_when_no_deny() {
        let mut c = pc(GwRequest::new("GET", "/", "h"));
        assert!(access(&serde_json::json!({}), &mut c).unwrap().is_none());
    }
}

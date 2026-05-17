// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! bot-detection plugin — block/allow known bot User-Agents.

use super::{GatewayPlugin, PluginCtx, PluginResult};
use async_trait::async_trait;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use regex::Regex;
use serde_json::Value;

pub struct BotDetectionPlugin;

// Default known-bad bots
const DEFAULT_DENY_PATTERNS: &[&str] = &[
    "(?i)wget",
    "(?i)curl",
    "(?i)libwww-perl",
    "(?i)python-requests",
    "(?i)scrapy",
    "(?i)nikto",
    "(?i)sqlmap",
    "(?i)nmap",
];

fn matches_any(ua: &str, patterns: &[&str]) -> bool {
    for p in patterns {
        if let Ok(re) = Regex::new(p) {
            if re.is_match(ua) {
                return true;
            }
        }
    }
    false
}

#[async_trait]
impl GatewayPlugin for BotDetectionPlugin {
    fn name(&self) -> &'static str {
        "bot-detection"
    }

    async fn access(&self, ctx: &mut PluginCtx, config: &Value) -> PluginResult {
        let ua = ctx.headers.get("user-agent").cloned().unwrap_or_default();

        let allow: Vec<&str> = config["allow"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();

        let deny: Vec<&str> = config["deny"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_else(|| DEFAULT_DENY_PATTERNS.to_vec());

        if !allow.is_empty() && matches_any(&ua, &allow) {
            return PluginResult::Continue;
        }

        if matches_any(&ua, &deny) {
            return PluginResult::Halt(
                (StatusCode::FORBIDDEN, axum::Json(serde_json::json!({"message": "Forbidden"}))).into_response(),
            );
        }

        PluginResult::Continue
    }
}

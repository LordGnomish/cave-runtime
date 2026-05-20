// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! ip-restriction plugin — allow/deny by CIDR or exact IP.

use super::{GatewayPlugin, PluginCtx, PluginResult};
use async_trait::async_trait;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use ipnet::IpNet;
use serde_json::Value;
use std::net::IpAddr;
use std::str::FromStr;

pub struct IpRestrictionPlugin;

fn ip_matches(ip_str: &str, patterns: &[&str]) -> bool {
    let ip = match IpAddr::from_str(ip_str) {
        Ok(a) => a,
        Err(_) => return false,
    };
    for p in patterns {
        if let Ok(net) = IpNet::from_str(p) {
            if net.contains(&ip) {
                return true;
            }
        } else if let Ok(exact) = IpAddr::from_str(p) {
            if exact == ip {
                return true;
            }
        }
    }
    false
}

#[async_trait]
impl GatewayPlugin for IpRestrictionPlugin {
    fn name(&self) -> &'static str {
        "ip-restriction"
    }

    async fn access(&self, ctx: &mut PluginCtx, config: &Value) -> PluginResult {
        let allow: Vec<&str> = config["allow"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();

        let deny: Vec<&str> = config["deny"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();

        // Respect X-Forwarded-For if configured
        let ip = ctx
            .headers
            .get("x-forwarded-for")
            .and_then(|v| v.split(',').next().map(str::trim).map(String::from))
            .unwrap_or_else(|| ctx.client_ip.clone());

        if !deny.is_empty() && ip_matches(&ip, &deny) {
            return PluginResult::Halt(
                (
                    StatusCode::FORBIDDEN,
                    axum::Json(serde_json::json!({"message": "Your IP address is not allowed"})),
                )
                    .into_response(),
            );
        }

        if !allow.is_empty() && !ip_matches(&ip, &allow) {
            return PluginResult::Halt(
                (
                    StatusCode::FORBIDDEN,
                    axum::Json(serde_json::json!({"message": "Your IP address is not allowed"})),
                )
                    .into_response(),
            );
        }

        PluginResult::Continue
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use serde_json::json;
    use std::collections::HashMap;

    fn ctx_with_ip(ip: &str) -> PluginCtx {
        PluginCtx::new(
            "GET".into(),
            "/".into(),
            HashMap::new(),
            Bytes::new(),
            ip.into(),
        )
    }

    #[tokio::test]
    async fn allows_permitted_ip() {
        let plugin = IpRestrictionPlugin;
        let mut ctx = ctx_with_ip("10.0.0.5");
        let config = json!({"allow": ["10.0.0.0/8"]});
        assert!(matches!(
            plugin.access(&mut ctx, &config).await,
            PluginResult::Continue
        ));
    }

    #[tokio::test]
    async fn blocks_denied_ip() {
        let plugin = IpRestrictionPlugin;
        let mut ctx = ctx_with_ip("192.168.1.1");
        let config = json!({"deny": ["192.168.0.0/16"]});
        assert!(matches!(
            plugin.access(&mut ctx, &config).await,
            PluginResult::Halt(_)
        ));
    }
}

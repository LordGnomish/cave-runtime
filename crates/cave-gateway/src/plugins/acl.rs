//! ACL plugin — allow/deny lists based on consumer groups.

use super::{GatewayPlugin, PluginCtx, PluginResult};
use async_trait::async_trait;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde_json::Value;

pub struct AclPlugin;

#[async_trait]
impl GatewayPlugin for AclPlugin {
    fn name(&self) -> &'static str {
        "acl"
    }

    async fn access(&self, ctx: &mut PluginCtx, config: &Value) -> PluginResult {
        // Consumer groups are expected to be set in ctx by a preceding auth plugin
        let consumer_groups: Vec<String> = ctx
            .ctx
            .get("consumer_groups")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();

        let allow: Vec<&str> = config["allow"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();

        let deny: Vec<&str> = config["deny"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();

        // Check deny list first
        if !deny.is_empty() {
            for group in &consumer_groups {
                if deny.contains(&group.as_str()) {
                    return PluginResult::Halt(
                        (StatusCode::FORBIDDEN, axum::Json(serde_json::json!({"message": "You cannot consume this service"}))).into_response(),
                    );
                }
            }
        }

        // Check allow list
        if !allow.is_empty() {
            let permitted = consumer_groups.iter().any(|g| allow.contains(&g.as_str()));
            if !permitted {
                return PluginResult::Halt(
                    (StatusCode::FORBIDDEN, axum::Json(serde_json::json!({"message": "You cannot consume this service"}))).into_response(),
                );
            }
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

    fn ctx_with_groups(groups: Vec<&str>) -> PluginCtx {
        let mut ctx = PluginCtx::new("GET".into(), "/".into(), HashMap::new(), Bytes::new(), "1.2.3.4".into());
        ctx.ctx.insert(
            "consumer_groups".to_string(),
            Value::Array(groups.into_iter().map(|g| Value::String(g.to_string())).collect()),
        );
        ctx
    }

    #[tokio::test]
    async fn allows_matching_group() {
        let plugin = AclPlugin;
        let mut ctx = ctx_with_groups(vec!["admin", "users"]);
        let config = json!({"allow": ["admin"]});
        assert!(matches!(plugin.access(&mut ctx, &config).await, PluginResult::Continue));
    }

    #[tokio::test]
    async fn blocks_non_matching_group() {
        let plugin = AclPlugin;
        let mut ctx = ctx_with_groups(vec!["guests"]);
        let config = json!({"allow": ["admin"]});
        assert!(matches!(plugin.access(&mut ctx, &config).await, PluginResult::Halt(_)));
    }

    #[tokio::test]
    async fn blocks_denied_group() {
        let plugin = AclPlugin;
        let mut ctx = ctx_with_groups(vec!["banned"]);
        let config = json!({"deny": ["banned"]});
        assert!(matches!(plugin.access(&mut ctx, &config).await, PluginResult::Halt(_)));
    }
}

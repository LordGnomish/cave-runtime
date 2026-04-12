//! request-transformer plugin — add/remove/rename/replace headers, query params, body fields.
//!
//! Config structure mirrors Kong's:
//! {
//!   "add":    { "headers": ["x-new:val"], "querystring": ["k:v"], "body": ["f:v"] },
//!   "replace":{ "headers": [...], "querystring": [...], "body": [...] },
//!   "remove": { "headers": ["x-old"], "querystring": ["k"], "body": ["f"] },
//!   "rename": { "headers": ["old:new"], "querystring": ["old:new"], "body": ["old:new"] },
//!   "append": { "headers": [...], "querystring": [...], "body": [...] },
//!   "allow":  { "body": ["allowed_field"] }
//! }

use super::{GatewayPlugin, PluginCtx, PluginResult};
use async_trait::async_trait;
use serde_json::Value;

pub struct RequestTransformerPlugin;

fn kv_split(s: &str) -> Option<(&str, &str)> {
    s.splitn(2, ':').collect::<Vec<_>>().as_slice().try_into().ok().map(|[k, v]: &[&str; 2]| (*k, *v))
}

fn apply_transforms(ctx: &mut PluginCtx, config: &Value) {
    // --- Headers ---
    // remove
    if let Some(arr) = config["remove"]["headers"].as_array() {
        for v in arr {
            if let Some(k) = v.as_str() {
                ctx.headers.remove(&k.to_lowercase());
            }
        }
    }
    // rename
    if let Some(arr) = config["rename"]["headers"].as_array() {
        for v in arr {
            if let Some(s) = v.as_str() {
                if let Some((old, new)) = kv_split(s) {
                    if let Some(val) = ctx.headers.remove(&old.to_lowercase()) {
                        ctx.headers.insert(new.to_lowercase(), val);
                    }
                }
            }
        }
    }
    // replace (only if key exists)
    if let Some(arr) = config["replace"]["headers"].as_array() {
        for v in arr {
            if let Some(s) = v.as_str() {
                if let Some((k, val)) = kv_split(s) {
                    if ctx.headers.contains_key(&k.to_lowercase()) {
                        ctx.headers.insert(k.to_lowercase(), val.to_string());
                    }
                }
            }
        }
    }
    // add (only if key does NOT exist)
    if let Some(arr) = config["add"]["headers"].as_array() {
        for v in arr {
            if let Some(s) = v.as_str() {
                if let Some((k, val)) = kv_split(s) {
                    ctx.headers.entry(k.to_lowercase()).or_insert_with(|| val.to_string());
                }
            }
        }
    }
    // append (always add, even if exists)
    if let Some(arr) = config["append"]["headers"].as_array() {
        for v in arr {
            if let Some(s) = v.as_str() {
                if let Some((k, val)) = kv_split(s) {
                    ctx.headers.insert(k.to_lowercase(), val.to_string());
                }
            }
        }
    }

    // --- Query string ---
    let mut query_pairs: Vec<(String, String)> = ctx
        .query
        .split('&')
        .filter(|s| !s.is_empty())
        .filter_map(|pair| {
            let mut kv = pair.splitn(2, '=');
            if let (Some(k), Some(v)) = (kv.next(), kv.next()) {
                Some((k.to_string(), v.to_string()))
            } else {
                None
            }
        })
        .collect();

    if let Some(arr) = config["remove"]["querystring"].as_array() {
        for v in arr {
            if let Some(k) = v.as_str() {
                query_pairs.retain(|(qk, _)| qk != k);
            }
        }
    }
    if let Some(arr) = config["add"]["querystring"].as_array() {
        for v in arr {
            if let Some(s) = v.as_str() {
                if let Some((k, val)) = kv_split(s) {
                    if !query_pairs.iter().any(|(qk, _)| qk == k) {
                        query_pairs.push((k.to_string(), val.to_string()));
                    }
                }
            }
        }
    }
    if let Some(arr) = config["replace"]["querystring"].as_array() {
        for v in arr {
            if let Some(s) = v.as_str() {
                if let Some((k, val)) = kv_split(s) {
                    for (qk, qv) in &mut query_pairs {
                        if qk == k {
                            *qv = val.to_string();
                        }
                    }
                }
            }
        }
    }

    ctx.query = query_pairs
        .iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect::<Vec<_>>()
        .join("&");

    // --- Body (JSON) ---
    if let Ok(mut json_body) = serde_json::from_slice::<Value>(&ctx.body) {
        if let Some(obj) = json_body.as_object_mut() {
            if let Some(arr) = config["remove"]["body"].as_array() {
                for v in arr {
                    if let Some(k) = v.as_str() {
                        obj.remove(k);
                    }
                }
            }
            if let Some(arr) = config["add"]["body"].as_array() {
                for v in arr {
                    if let Some(s) = v.as_str() {
                        if let Some((k, val)) = kv_split(s) {
                            obj.entry(k).or_insert_with(|| Value::String(val.to_string()));
                        }
                    }
                }
            }
            if let Some(arr) = config["replace"]["body"].as_array() {
                for v in arr {
                    if let Some(s) = v.as_str() {
                        if let Some((k, val)) = kv_split(s) {
                            if obj.contains_key(k) {
                                obj.insert(k.to_string(), Value::String(val.to_string()));
                            }
                        }
                    }
                }
            }
            if let Some(arr) = config["rename"]["body"].as_array() {
                for v in arr {
                    if let Some(s) = v.as_str() {
                        if let Some((old, new)) = kv_split(s) {
                            if let Some(val) = obj.remove(old) {
                                obj.insert(new.to_string(), val);
                            }
                        }
                    }
                }
            }
            // allow list — drop everything else
            if let Some(arr) = config["allow"]["body"].as_array() {
                let allowed: Vec<&str> = arr.iter().filter_map(|v| v.as_str()).collect();
                if !allowed.is_empty() {
                    obj.retain(|k, _| allowed.contains(&k.as_str()));
                }
            }

            if let Ok(new_body) = serde_json::to_vec(obj) {
                ctx.body = bytes::Bytes::from(new_body);
            }
        }
    }
}

#[async_trait]
impl GatewayPlugin for RequestTransformerPlugin {
    fn name(&self) -> &'static str {
        "request-transformer"
    }

    async fn access(&self, ctx: &mut PluginCtx, config: &Value) -> PluginResult {
        apply_transforms(ctx, config);
        PluginResult::Continue
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use serde_json::json;
    use std::collections::HashMap;

    #[tokio::test]
    async fn adds_header() {
        let plugin = RequestTransformerPlugin;
        let mut ctx = PluginCtx::new("GET".into(), "/".into(), HashMap::new(), Bytes::new(), "1.2.3.4".into());
        let config = json!({"add": {"headers": ["x-env:production"]}});
        plugin.access(&mut ctx, &config).await;
        assert_eq!(ctx.headers.get("x-env").map(String::as_str), Some("production"));
    }

    #[tokio::test]
    async fn removes_header() {
        let plugin = RequestTransformerPlugin;
        let mut headers = HashMap::new();
        headers.insert("x-secret".to_string(), "hidden".to_string());
        let mut ctx = PluginCtx::new("GET".into(), "/".into(), headers, Bytes::new(), "1.2.3.4".into());
        let config = json!({"remove": {"headers": ["x-secret"]}});
        plugin.access(&mut ctx, &config).await;
        assert!(!ctx.headers.contains_key("x-secret"));
    }

    #[tokio::test]
    async fn transforms_json_body() {
        let plugin = RequestTransformerPlugin;
        let body = Bytes::from(r#"{"name":"alice","password":"secret"}"#);
        let mut headers = HashMap::new();
        headers.insert("content-type".to_string(), "application/json".to_string());
        let mut ctx = PluginCtx::new("POST".into(), "/".into(), headers, body, "1.2.3.4".into());
        let config = json!({"remove": {"body": ["password"]}});
        plugin.access(&mut ctx, &config).await;
        let result: Value = serde_json::from_slice(&ctx.body).unwrap();
        assert!(result.get("password").is_none());
        assert_eq!(result["name"].as_str(), Some("alice"));
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! grpc-gateway plugin — REST→gRPC transcoding.
//!
//! Converts HTTP/JSON requests to gRPC binary frames and back.
//! Config:
//!   proto_path: path to .proto file (or inline descriptor)
//!   service: "mypackage.MyService"

use super::{GatewayPlugin, PluginCtx, PluginResult};
use async_trait::async_trait;
use serde_json::Value;

pub struct GrpcGatewayPlugin;

#[async_trait]
impl GatewayPlugin for GrpcGatewayPlugin {
    fn name(&self) -> &'static str {
        "grpc-gateway"
    }

    async fn access(&self, ctx: &mut PluginCtx, config: &Value) -> PluginResult {
        // Signal to proxy handler to use gRPC encoding
        ctx.ctx
            .insert("grpc_gateway_enabled".to_string(), Value::Bool(true));

        // Rewrite content-type for upstream
        ctx.headers
            .insert("content-type".to_string(), "application/grpc".to_string());
        ctx.headers.insert("te".to_string(), "trailers".to_string());

        // Map HTTP method+path to gRPC method
        // e.g. POST /v1/hello → /helloworld.Greeter/SayHello
        if let Some(path_map) = config["path_map"].as_object() {
            let key = format!("{}:{}", ctx.method, ctx.path);
            if let Some(grpc_method) = path_map.get(&key).and_then(|v| v.as_str()) {
                ctx.ctx.insert(
                    "grpc_method".to_string(),
                    Value::String(grpc_method.to_string()),
                );
            }
        }

        PluginResult::Continue
    }
}

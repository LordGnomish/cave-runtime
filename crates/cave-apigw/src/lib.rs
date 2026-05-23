// SPDX-License-Identifier: AGPL-3.0-or-later
//
// cave-apigw — API Gateway
//
// Reimplementation of:
//   - Kong/kong v3.9.1 (Apache-2.0)            — routes, services, upstreams, plugin model, Admin API, decK
//   - envoyproxy/envoy v1.38.0 (Apache-2.0)    — HTTP/2/3 + gRPC routing + transcoding reference
//
// Source SHAs pinned in parity.manifest.toml; NOTICE attribution required at workspace root.

pub mod error;
pub mod models;
pub mod store;
pub mod router;
pub mod matcher;
pub mod lb;
pub mod health;
pub mod proxy;
pub mod http1;
pub mod http2;
pub mod http3;
pub mod grpc;
pub mod websocket;
pub mod tls;
pub mod pqc;
pub mod acme_hook;
pub mod admin;
pub mod declarative;
pub mod consumer;
pub mod metrics;
pub mod tracing_otel;
pub mod access_log;
pub mod cli;
pub mod plugins;
pub mod crd;

pub use error::{AGwError, AGwResult};
pub use models::{
    Consumer, GwConfig, Plugin, PluginKind, Protocol, Route, Service, Target, Upstream,
    UpstreamAlgorithm,
};
pub use store::GwStore;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn default_config() -> GwConfig {
    GwConfig::default()
}

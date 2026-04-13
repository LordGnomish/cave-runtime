//! Factory function for selecting the trace backend from config.

use std::sync::Arc;

use crate::backend::{BuiltinTraceBackend, TraceBackend, TraceBackendProfile};
use crate::adapters::{
    datadog::DatadogApmAdapter,
    jaeger_remote::JaegerRemoteAdapter,
    newrelic::NewRelicTraceAdapter,
};
use crate::TraceState;

/// Instantiate the correct trace backend for the given profile.
pub fn create_trace_backend(
    profile: TraceBackendProfile,
    state: Arc<TraceState>,
) -> Arc<dyn TraceBackend> {
    match profile {
        TraceBackendProfile::Builtin => {
            tracing::info!(backend = "builtin-tempo", "trace backend selected");
            Arc::new(BuiltinTraceBackend::new(state))
        }

        TraceBackendProfile::Datadog => {
            let config = crate::adapters::datadog::DatadogApmConfig {
                api_key: std::env::var("DD_API_KEY").unwrap_or_default(),
                site: std::env::var("DD_SITE").unwrap_or_else(|_| "datadoghq.com".to_string()),
                agent_url: std::env::var("DD_AGENT_URL").ok(),
                service: std::env::var("DD_SERVICE")
                    .unwrap_or_else(|_| "cave-runtime".to_string()),
                env: std::env::var("DD_ENV")
                    .unwrap_or_else(|_| "production".to_string()),
            };
            tracing::info!(backend = "datadog-apm", site = %config.site, "trace backend selected");
            Arc::new(DatadogApmAdapter::new(config))
        }

        TraceBackendProfile::Jaeger => {
            let config = crate::adapters::jaeger_remote::JaegerRemoteConfig {
                collector_url: std::env::var("JAEGER_COLLECTOR_URL")
                    .unwrap_or_else(|_| "http://jaeger-collector:9411/api/v2/spans".to_string()),
                query_url: std::env::var("JAEGER_QUERY_URL").ok(),
            };
            tracing::info!(backend = "jaeger-remote", url = %config.collector_url, "trace backend selected");
            Arc::new(JaegerRemoteAdapter::new(config))
        }

        TraceBackendProfile::NewRelic => {
            let config = crate::adapters::newrelic::NewRelicTraceConfig {
                license_key: std::env::var("NR_LICENSE_KEY").unwrap_or_default(),
                account_id: std::env::var("NR_ACCOUNT_ID").ok().and_then(|s| s.parse().ok()),
                region: std::env::var("NR_REGION").unwrap_or_else(|_| "US".to_string()),
            };
            tracing::info!(backend = "new-relic-traces", region = %config.region, "trace backend selected");
            Arc::new(NewRelicTraceAdapter::new(config))
        }
    }
}

/// Convenience: build backend from environment variables alone.
///
/// `CAVE_TRACE_BACKEND` = `builtin` | `datadog` | `jaeger` | `new_relic`
pub fn create_trace_backend_from_env(state: Arc<TraceState>) -> Arc<dyn TraceBackend> {
    let profile = match std::env::var("CAVE_TRACE_BACKEND")
        .unwrap_or_else(|_| "builtin".to_string())
        .as_str()
    {
        "datadog" => TraceBackendProfile::Datadog,
        "jaeger" => TraceBackendProfile::Jaeger,
        "new_relic" | "newrelic" => TraceBackendProfile::NewRelic,
        _ => TraceBackendProfile::Builtin,
    };
    create_trace_backend(profile, state)
}

//! Factory function for selecting the metrics backend from config.

use std::sync::Arc;

use crate::backend::{BuiltinMetricsBackend, MetricsBackend, MetricsBackendProfile};
use crate::adapters::{
    datadog::DatadogAdapter,
    newrelic::NewRelicAdapter,
    cloudwatch::CloudWatchAdapter,
};
use crate::state::MetricsState;

/// Instantiate the correct metrics backend for the given profile.
///
/// The built-in TSDB backend requires a fully initialised [`MetricsState`].
/// External adapters configure themselves from environment variables.
pub fn create_metrics_backend(
    profile: MetricsBackendProfile,
    state: Arc<MetricsState>,
) -> Arc<dyn MetricsBackend> {
    match profile {
        MetricsBackendProfile::Builtin => {
            tracing::info!(backend = "builtin-tsdb", "metrics backend selected");
            Arc::new(BuiltinMetricsBackend::new(state))
        }

        MetricsBackendProfile::Datadog => {
            let config = crate::adapters::datadog::DatadogConfig {
                api_key: std::env::var("DD_API_KEY").unwrap_or_default(),
                site: std::env::var("DD_SITE").unwrap_or_else(|_| "datadoghq.com".to_string()),
            };
            tracing::info!(backend = "datadog", site = %config.site, "metrics backend selected");
            Arc::new(DatadogAdapter::new(config))
        }

        MetricsBackendProfile::NewRelic => {
            let config = crate::adapters::newrelic::NewRelicConfig {
                license_key: std::env::var("NR_LICENSE_KEY").unwrap_or_default(),
                account_id: std::env::var("NR_ACCOUNT_ID").unwrap_or_default(),
                region: std::env::var("NR_REGION").unwrap_or_else(|_| "US".to_string()),
            };
            tracing::info!(backend = "new-relic", region = %config.region, "metrics backend selected");
            Arc::new(NewRelicAdapter::new(config))
        }

        MetricsBackendProfile::CloudWatch => {
            let config = crate::adapters::cloudwatch::CloudWatchConfig {
                namespace: std::env::var("CW_NAMESPACE")
                    .unwrap_or_else(|_| "CaveRuntime".to_string()),
                region: std::env::var("AWS_REGION")
                    .or_else(|_| std::env::var("AWS_DEFAULT_REGION"))
                    .unwrap_or_else(|_| "us-east-1".to_string()),
            };
            tracing::info!(backend = "cloudwatch", namespace = %config.namespace, "metrics backend selected");
            Arc::new(CloudWatchAdapter::new(config))
        }
    }
}

/// Convenience: build backend from environment variables alone.
///
/// `CAVE_METRICS_BACKEND` = `builtin` | `datadog` | `new_relic` | `cloudwatch`
pub fn create_metrics_backend_from_env(state: Arc<MetricsState>) -> Arc<dyn MetricsBackend> {
    let profile = match std::env::var("CAVE_METRICS_BACKEND")
        .unwrap_or_else(|_| "builtin".to_string())
        .as_str()
    {
        "datadog" => MetricsBackendProfile::Datadog,
        "new_relic" | "newrelic" => MetricsBackendProfile::NewRelic,
        "cloudwatch" => MetricsBackendProfile::CloudWatch,
        _ => MetricsBackendProfile::Builtin,
    };
    create_metrics_backend(profile, state)
}

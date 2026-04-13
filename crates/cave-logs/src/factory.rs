//! Factory function for selecting the logs backend from config.

use std::sync::Arc;

use crate::backend::{BuiltinLogsBackend, LogsBackend, LogsBackendProfile};
use crate::adapters::{
    splunk::SplunkAdapter,
    datadog::DatadogLogsAdapter,
    cloudwatch::CloudWatchLogsAdapter,
};
use crate::store::LogStore;

/// Instantiate the correct logs backend for the given profile.
pub fn create_logs_backend(
    profile: LogsBackendProfile,
    store: Arc<LogStore>,
) -> Arc<dyn LogsBackend> {
    match profile {
        LogsBackendProfile::Builtin => {
            tracing::info!(backend = "builtin-loki", "logs backend selected");
            Arc::new(BuiltinLogsBackend::new(store))
        }

        LogsBackendProfile::Splunk => {
            let config = crate::adapters::splunk::SplunkConfig {
                hec_url: std::env::var("SPLUNK_HEC_URL").unwrap_or_default(),
                hec_token: std::env::var("SPLUNK_HEC_TOKEN").unwrap_or_default(),
                index: std::env::var("SPLUNK_INDEX")
                    .unwrap_or_else(|_| "cave_logs".to_string()),
                sourcetype: std::env::var("SPLUNK_SOURCETYPE")
                    .unwrap_or_else(|_| "cave:runtime".to_string()),
            };
            tracing::info!(backend = "splunk", hec_url = %config.hec_url, "logs backend selected");
            Arc::new(SplunkAdapter::new(config))
        }

        LogsBackendProfile::Datadog => {
            let config = crate::adapters::datadog::DatadogLogsConfig {
                api_key: std::env::var("DD_API_KEY").unwrap_or_default(),
                site: std::env::var("DD_SITE").unwrap_or_else(|_| "datadoghq.com".to_string()),
                service: std::env::var("DD_SERVICE")
                    .unwrap_or_else(|_| "cave-runtime".to_string()),
            };
            tracing::info!(backend = "datadog-logs", site = %config.site, "logs backend selected");
            Arc::new(DatadogLogsAdapter::new(config))
        }

        LogsBackendProfile::CloudWatch => {
            let config = crate::adapters::cloudwatch::CloudWatchLogsConfig {
                log_group: std::env::var("CW_LOG_GROUP")
                    .unwrap_or_else(|_| "/cave/runtime".to_string()),
                log_stream: std::env::var("CW_LOG_STREAM")
                    .unwrap_or_else(|_| "default".to_string()),
                region: std::env::var("AWS_REGION")
                    .or_else(|_| std::env::var("AWS_DEFAULT_REGION"))
                    .unwrap_or_else(|_| "us-east-1".to_string()),
            };
            tracing::info!(backend = "cloudwatch-logs", log_group = %config.log_group, "logs backend selected");
            Arc::new(CloudWatchLogsAdapter::new(config))
        }
    }
}

/// Convenience: build backend from environment variables alone.
///
/// `CAVE_LOGS_BACKEND` = `builtin` | `splunk` | `datadog` | `cloudwatch`
pub fn create_logs_backend_from_env(store: Arc<LogStore>) -> Arc<dyn LogsBackend> {
    let profile = match std::env::var("CAVE_LOGS_BACKEND")
        .unwrap_or_else(|_| "builtin".to_string())
        .as_str()
    {
        "splunk" => LogsBackendProfile::Splunk,
        "datadog" => LogsBackendProfile::Datadog,
        "cloudwatch" => LogsBackendProfile::CloudWatch,
        _ => LogsBackendProfile::Builtin,
    };
    create_logs_backend(profile, store)
}

//! Telemetry API — Istio Telemetry resource manager.
//!
//! Controls per-workload metrics, access logging, and tracing configuration.
//! Matches Istio's telemetry.istio.io/v1 API.

use crate::models::Telemetry;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};
use tracing::debug;

// ─────────────────────────────────────────────────────────────
// TelemetryManager
// ─────────────────────────────────────────────────────────────

/// Manages Telemetry resources per namespace.
#[derive(Debug, Clone)]
pub struct TelemetryManager {
    /// Keyed by "namespace/name"
    resources: Arc<RwLock<HashMap<String, Telemetry>>>,
}

impl Default for TelemetryManager {
    fn default() -> Self {
        Self::new()
    }
}

impl TelemetryManager {
    pub fn new() -> Self {
        Self { resources: Arc::new(RwLock::new(HashMap::new())) }
    }

    pub fn upsert(&self, t: Telemetry) {
        let key = format!("{}/{}", t.namespace, t.name);
        self.resources.write().unwrap().insert(key, t);
    }

    pub fn remove(&self, namespace: &str, name: &str) {
        let key = format!("{namespace}/{name}");
        self.resources.write().unwrap().remove(&key);
    }

    pub fn list(&self) -> Vec<Telemetry> {
        self.resources.read().unwrap().values().cloned().collect()
    }

    pub fn get(&self, namespace: &str, name: &str) -> Option<Telemetry> {
        let key = format!("{namespace}/{name}");
        self.resources.read().unwrap().get(&key).cloned()
    }

    /// Resolve the effective Telemetry for a workload (namespace + labels).
    ///
    /// Priority: workload-specific > namespace-wide > root namespace.
    pub fn effective_telemetry(
        &self,
        namespace: &str,
        workload_labels: &HashMap<String, String>,
    ) -> Option<Telemetry> {
        let map = self.resources.read().unwrap();

        let mut workload_match: Option<Telemetry> = None;
        let mut namespace_match: Option<Telemetry> = None;
        let mut root_match: Option<Telemetry> = None;

        for t in map.values() {
            if t.namespace == namespace {
                let is_namespace_wide =
                    t.selector.as_ref().map(|s| s.is_empty()).unwrap_or(true);
                if is_namespace_wide {
                    namespace_match = Some(t.clone());
                } else if let Some(sel) = &t.selector {
                    let matches = sel.iter().all(|(k, v)| {
                        workload_labels.get(k).map(|vv| vv == v).unwrap_or(false)
                    });
                    if matches {
                        workload_match = Some(t.clone());
                    }
                }
            } else if t.namespace == "istio-system" || t.namespace == "cave-system" {
                root_match = Some(t.clone());
            }
        }

        let result = workload_match.or(namespace_match).or(root_match);
        if let Some(ref t) = result {
            debug!(namespace = %namespace, telemetry = %t.name, "Resolved effective telemetry");
        }
        result
    }

    /// Check whether access logging is enabled for a workload.
    pub fn access_logging_enabled(
        &self,
        namespace: &str,
        workload_labels: &HashMap<String, String>,
    ) -> bool {
        self.effective_telemetry(namespace, workload_labels)
            .map(|t| {
                t.access_logging
                    .iter()
                    .any(|al| al.disabled.map(|d| !d).unwrap_or(true) && !al.providers.is_empty())
            })
            .unwrap_or(false)
    }

    /// Get the sampling rate for a workload (None = use default).
    pub fn tracing_sampling_rate(
        &self,
        namespace: &str,
        workload_labels: &HashMap<String, String>,
    ) -> Option<f64> {
        self.effective_telemetry(namespace, workload_labels)
            .and_then(|t| t.tracing.first().and_then(|tr| tr.random_sampling_percentage))
    }

    /// Snapshot of all Telemetry resources for observability.
    pub fn snapshot(&self) -> TelemetrySnapshot {
        let resources = self.resources.read().unwrap();
        let count = resources.len();
        let namespaces: std::collections::HashSet<_> =
            resources.values().map(|t| t.namespace.clone()).collect();
        TelemetrySnapshot { total_resources: count, namespaces: namespaces.into_iter().collect() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetrySnapshot {
    pub total_resources: usize,
    pub namespaces: Vec<String>,
}

// ─────────────────────────────────────────────────────────────
// Access log format builder (Envoy-compatible)
// ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessLogFormat {
    pub format: AccessLogFormatType,
    pub fields: Vec<AccessLogField>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum AccessLogFormatType {
    Json,
    Text,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessLogField {
    pub name: String,
    pub value: AccessLogFieldValue,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AccessLogFieldValue {
    /// Envoy command operator (e.g. "%REQ(:path)%").
    EnvoyOperator(String),
    /// Static literal string.
    Literal(String),
    /// Metadata key.
    Metadata { filter: String, path: Vec<String> },
}

impl AccessLogFormat {
    /// Default Istio-style JSON access log format.
    pub fn default_json() -> Self {
        Self {
            format: AccessLogFormatType::Json,
            fields: vec![
                AccessLogField {
                    name: "start_time".to_string(),
                    value: AccessLogFieldValue::EnvoyOperator("%START_TIME%".to_string()),
                },
                AccessLogField {
                    name: "method".to_string(),
                    value: AccessLogFieldValue::EnvoyOperator("%REQ(:METHOD)%".to_string()),
                },
                AccessLogField {
                    name: "path".to_string(),
                    value: AccessLogFieldValue::EnvoyOperator("%REQ(X-ENVOY-ORIGINAL-PATH?:PATH)%".to_string()),
                },
                AccessLogField {
                    name: "protocol".to_string(),
                    value: AccessLogFieldValue::EnvoyOperator("%PROTOCOL%".to_string()),
                },
                AccessLogField {
                    name: "response_code".to_string(),
                    value: AccessLogFieldValue::EnvoyOperator("%RESPONSE_CODE%".to_string()),
                },
                AccessLogField {
                    name: "response_flags".to_string(),
                    value: AccessLogFieldValue::EnvoyOperator("%RESPONSE_FLAGS%".to_string()),
                },
                AccessLogField {
                    name: "bytes_received".to_string(),
                    value: AccessLogFieldValue::EnvoyOperator("%BYTES_RECEIVED%".to_string()),
                },
                AccessLogField {
                    name: "bytes_sent".to_string(),
                    value: AccessLogFieldValue::EnvoyOperator("%BYTES_SENT%".to_string()),
                },
                AccessLogField {
                    name: "duration".to_string(),
                    value: AccessLogFieldValue::EnvoyOperator("%DURATION%".to_string()),
                },
                AccessLogField {
                    name: "upstream_service_time".to_string(),
                    value: AccessLogFieldValue::EnvoyOperator(
                        "%RESP(X-ENVOY-UPSTREAM-SERVICE-TIME)%".to_string(),
                    ),
                },
                AccessLogField {
                    name: "x_forwarded_for".to_string(),
                    value: AccessLogFieldValue::EnvoyOperator(
                        "%REQ(X-FORWARDED-FOR)%".to_string(),
                    ),
                },
                AccessLogField {
                    name: "user_agent".to_string(),
                    value: AccessLogFieldValue::EnvoyOperator("%REQ(USER-AGENT)%".to_string()),
                },
                AccessLogField {
                    name: "request_id".to_string(),
                    value: AccessLogFieldValue::EnvoyOperator("%REQ(X-REQUEST-ID)%".to_string()),
                },
                AccessLogField {
                    name: "authority".to_string(),
                    value: AccessLogFieldValue::EnvoyOperator("%REQ(:AUTHORITY)%".to_string()),
                },
                AccessLogField {
                    name: "upstream_host".to_string(),
                    value: AccessLogFieldValue::EnvoyOperator("%UPSTREAM_HOST%".to_string()),
                },
                AccessLogField {
                    name: "upstream_cluster".to_string(),
                    value: AccessLogFieldValue::EnvoyOperator("%UPSTREAM_CLUSTER%".to_string()),
                },
                AccessLogField {
                    name: "upstream_local_address".to_string(),
                    value: AccessLogFieldValue::EnvoyOperator(
                        "%UPSTREAM_LOCAL_ADDRESS%".to_string(),
                    ),
                },
                AccessLogField {
                    name: "downstream_local_address".to_string(),
                    value: AccessLogFieldValue::EnvoyOperator(
                        "%DOWNSTREAM_LOCAL_ADDRESS%".to_string(),
                    ),
                },
                AccessLogField {
                    name: "downstream_remote_address".to_string(),
                    value: AccessLogFieldValue::EnvoyOperator(
                        "%DOWNSTREAM_REMOTE_ADDRESS%".to_string(),
                    ),
                },
                AccessLogField {
                    name: "trace_id".to_string(),
                    value: AccessLogFieldValue::EnvoyOperator(
                        "%REQ(X-B3-TRACEID)%".to_string(),
                    ),
                },
            ],
        }
    }
}

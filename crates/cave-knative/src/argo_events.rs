// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Argo Events parity — `argoproj/argo-events v1.9.10`
//! (`pkg/apis/{eventsource,sensor,eventbus}/v1alpha1/types.go`).
//!
//! Three CRDs:
//! * [`EventSource`]  — emits events into the EventBus (webhook / kafka /
//!                      github / k8s-resource / generic / pulsar / sqs).
//! * [`Sensor`]       — subscribes to events, applies filters, fires Triggers.
//! * [`EventBus`]     — pub-sub transport (JetStream / Kafka backends).
//!
//! Filters + triggers are pure-function — the I/O side (HTTP poll, Kafka
//! consumer, k8s apply) is the caller's responsibility. The module ships
//! the schema + reducers + a sample evaluator.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

// ─── EventSource ────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EventSource {
    pub id: Uuid,
    pub name: String,
    pub namespace: String,
    pub event_bus_name: String,
    pub spec: EventSourceSpec,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct EventSourceSpec {
    #[serde(default)]
    pub webhook: BTreeMap<String, WebhookSourceCfg>,
    #[serde(default)]
    pub kafka: BTreeMap<String, KafkaSourceCfg>,
    #[serde(default)]
    pub github: BTreeMap<String, GithubSourceCfg>,
    #[serde(default)]
    pub generic: BTreeMap<String, GenericSourceCfg>,
    #[serde(default)]
    pub k8s_resource: BTreeMap<String, K8sResourceSourceCfg>,
    #[serde(default)]
    pub pulsar: BTreeMap<String, PulsarSourceCfg>,
    #[serde(default)]
    pub sqs: BTreeMap<String, SqsSourceCfg>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WebhookSourceCfg {
    pub endpoint: String,
    pub method: String,
    pub port: u16,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KafkaSourceCfg {
    pub brokers: Vec<String>,
    pub topic: String,
    pub consumer_group: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GithubSourceCfg {
    pub repositories: Vec<String>,
    pub webhook_secret_ref: String,
    pub events: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GenericSourceCfg {
    pub url: String,
    pub auth_secret_ref: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct K8sResourceSourceCfg {
    pub group: String,
    pub version: String,
    pub resource: String,
    pub namespace: String,
    pub event_types: Vec<String>, // ADD / UPDATE / DELETE
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PulsarSourceCfg {
    pub tenant: String,
    pub namespace: String,
    pub topics: Vec<String>,
    pub subscription: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SqsSourceCfg {
    pub region: String,
    pub queue: String,
    pub access_key_ref: Option<String>,
}

// ─── Sensor ─────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Sensor {
    pub id: Uuid,
    pub name: String,
    pub namespace: String,
    pub event_bus_name: String,
    pub spec: SensorSpec,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SensorSpec {
    pub dependencies: Vec<EventDependency>,
    pub triggers: Vec<Trigger>,
    #[serde(default)]
    pub error_on_failed_round: bool,
}

/// A subscription to one EventSource — optionally filtered.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EventDependency {
    pub name: String,
    pub event_source_name: String,
    pub event_name: String,
    #[serde(default)]
    pub filters: Filters,
}

/// All three Argo-supported filter kinds.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Filters {
    #[serde(default)]
    pub time: Option<TimeFilter>,
    #[serde(default)]
    pub context: Option<ContextFilter>,
    #[serde(default)]
    pub data: Vec<DataFilter>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TimeFilter {
    /// HH:MM:SS in UTC — accept events with timestamp inside the window.
    pub start: String,
    pub stop: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ContextFilter {
    #[serde(default)]
    pub r#type: Option<String>,
    #[serde(default)]
    pub subject: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DataFilter {
    /// JSON pointer (`/foo/bar`) into the event payload.
    pub path: String,
    /// Expected literal type — `string` / `number` / `bool`.
    pub data_type: String,
    /// Accepted values (logical OR).
    pub value: Vec<String>,
    #[serde(default)]
    pub comparator: Option<String>, // `=`, `!=`, `<`, `>`, `<=`, `>=`
}

// ─── Trigger ────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Trigger {
    pub name: String,
    pub template: TriggerTemplate,
    #[serde(default)]
    pub retry_strategy: Option<TriggerRetry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum TriggerTemplate {
    K8s {
        group: String,
        version: String,
        resource: String,
        operation: String, // create / update / patch / delete
        source: serde_json::Value,
    },
    Http {
        url: String,
        method: String,
        payload: serde_json::Value,
    },
    ArgoWorkflow {
        operation: String, // submit / resubmit / resume / suspend / retry / terminate / stop
        source: serde_json::Value,
    },
    Kafka {
        url: String,
        topic: String,
        partition: Option<i32>,
        payload: serde_json::Value,
    },
    Slack {
        channel: String,
        message: String,
        webhook_url_secret_ref: String,
    },
    Pulsar {
        url: String,
        topic: String,
        payload: serde_json::Value,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TriggerRetry {
    pub steps: u32,
    pub duration: String,
    #[serde(default)]
    pub factor: Option<u32>,
}

// ─── EventBus ───────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EventBus {
    pub id: Uuid,
    pub name: String,
    pub namespace: String,
    pub backend: EventBusBackend,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum EventBusBackend {
    JetStream {
        version: String,
        replicas: u32,
    },
    Kafka {
        brokers: Vec<String>,
        topic_prefix: String,
    },
}

// ─── Event + filter evaluation ──────────────────────────────────────────────

/// CloudEvents-compatible envelope carried over the EventBus.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Event {
    pub id: String,
    pub source: String,
    pub r#type: String,
    pub subject: String,
    pub time: DateTime<Utc>,
    pub data: serde_json::Value,
}

/// Pure-function filter evaluation. Returns true if the event passes all
/// of `filters`. Used by the sensor reducer to decide whether to fire.
pub fn matches_filters(ev: &Event, f: &Filters) -> bool {
    if let Some(ctx) = &f.context {
        if ctx.r#type.as_deref().is_some_and(|t| t != ev.r#type) {
            return false;
        }
        if ctx
            .subject
            .as_deref()
            .is_some_and(|s| s != ev.subject)
        {
            return false;
        }
        if ctx.source.as_deref().is_some_and(|s| s != ev.source) {
            return false;
        }
    }
    if let Some(t) = &f.time {
        let hms = ev.time.format("%H:%M:%S").to_string();
        if hms.as_str() < t.start.as_str() || hms.as_str() > t.stop.as_str() {
            return false;
        }
    }
    for d in &f.data {
        let Some(v) = json_pointer_get(&ev.data, &d.path) else {
            return false;
        };
        if !data_filter_passes(v, d) {
            return false;
        }
    }
    true
}

fn json_pointer_get<'a>(root: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    root.pointer(path)
}

fn data_filter_passes(v: &serde_json::Value, f: &DataFilter) -> bool {
    let cmp = f.comparator.as_deref().unwrap_or("=");
    let to_str = |x: &serde_json::Value| -> Option<String> {
        match x {
            serde_json::Value::String(s) => Some(s.clone()),
            serde_json::Value::Number(n) => Some(n.to_string()),
            serde_json::Value::Bool(b) => Some(b.to_string()),
            _ => None,
        }
    };
    let lhs = match to_str(v) {
        Some(s) => s,
        None => return false,
    };
    f.value.iter().any(|expected| match cmp {
        "=" => &lhs == expected,
        "!=" => &lhs != expected,
        "<" | ">" | "<=" | ">=" => {
            let (Ok(a), Ok(b)) = (lhs.parse::<f64>(), expected.parse::<f64>()) else {
                return false;
            };
            match cmp {
                "<" => a < b,
                ">" => a > b,
                "<=" => a <= b,
                ">=" => a >= b,
                _ => false,
            }
        }
        _ => false,
    })
}

/// Take a Sensor + an incoming event; return the list of triggers it should
/// fire. Pure function — no side effects.
pub fn evaluate_sensor(sensor: &Sensor, ev: &Event, dep_name: &str) -> Vec<Trigger> {
    let Some(dep) = sensor.spec.dependencies.iter().find(|d| d.name == dep_name) else {
        return Vec::new();
    };
    if !matches_filters(ev, &dep.filters) {
        return Vec::new();
    }
    sensor.spec.triggers.clone()
}

impl EventSource {
    pub fn new(
        name: impl Into<String>,
        namespace: impl Into<String>,
        event_bus_name: impl Into<String>,
        spec: EventSourceSpec,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            namespace: namespace.into(),
            event_bus_name: event_bus_name.into(),
            spec,
            created_at: Utc::now(),
        }
    }
}

impl Sensor {
    pub fn new(
        name: impl Into<String>,
        namespace: impl Into<String>,
        event_bus_name: impl Into<String>,
        spec: SensorSpec,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            namespace: namespace.into(),
            event_bus_name: event_bus_name.into(),
            spec,
            created_at: Utc::now(),
        }
    }
}

impl EventBus {
    pub fn new(
        name: impl Into<String>,
        namespace: impl Into<String>,
        backend: EventBusBackend,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            namespace: namespace.into(),
            backend,
            created_at: Utc::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(typ: &str, payload: serde_json::Value) -> Event {
        Event {
            id: "1".into(),
            source: "argo".into(),
            r#type: typ.into(),
            subject: "x".into(),
            time: Utc::now(),
            data: payload,
        }
    }

    #[test]
    fn event_source_with_webhook_round_trips() {
        let mut spec = EventSourceSpec::default();
        spec.webhook.insert(
            "github".into(),
            WebhookSourceCfg {
                endpoint: "/webhook".into(),
                method: "POST".into(),
                port: 12000,
            },
        );
        let es = EventSource::new("github", "argo", "default", spec);
        let j = serde_json::to_string(&es).unwrap();
        let back: EventSource = serde_json::from_str(&j).unwrap();
        assert_eq!(back.spec.webhook.len(), 1);
    }

    #[test]
    fn context_filter_rejects_mismatch_type() {
        let f = Filters {
            time: None,
            context: Some(ContextFilter {
                r#type: Some("ci.passed".into()),
                ..Default::default()
            }),
            data: vec![],
        };
        assert!(!matches_filters(&ev("ci.failed", serde_json::json!({})), &f));
    }

    #[test]
    fn context_filter_accepts_match() {
        let f = Filters {
            time: None,
            context: Some(ContextFilter {
                r#type: Some("ci.passed".into()),
                ..Default::default()
            }),
            data: vec![],
        };
        assert!(matches_filters(&ev("ci.passed", serde_json::json!({})), &f));
    }

    #[test]
    fn data_filter_string_equality_accepts() {
        let f = Filters {
            data: vec![DataFilter {
                path: "/build/status".into(),
                data_type: "string".into(),
                value: vec!["success".into()],
                comparator: Some("=".into()),
            }],
            ..Default::default()
        };
        assert!(matches_filters(
            &ev("e", serde_json::json!({"build":{"status":"success"}})),
            &f,
        ));
    }

    #[test]
    fn data_filter_numeric_comparator() {
        let f = Filters {
            data: vec![DataFilter {
                path: "/count".into(),
                data_type: "number".into(),
                value: vec!["5".into()],
                comparator: Some(">=".into()),
            }],
            ..Default::default()
        };
        assert!(matches_filters(
            &ev("e", serde_json::json!({"count": 7})),
            &f,
        ));
        assert!(!matches_filters(
            &ev("e", serde_json::json!({"count": 3})),
            &f,
        ));
    }

    #[test]
    fn data_filter_missing_path_fails() {
        let f = Filters {
            data: vec![DataFilter {
                path: "/missing".into(),
                data_type: "string".into(),
                value: vec!["x".into()],
                comparator: None,
            }],
            ..Default::default()
        };
        assert!(!matches_filters(
            &ev("e", serde_json::json!({"present": "x"})),
            &f,
        ));
    }

    #[test]
    fn sensor_evaluation_fires_triggers_when_dep_passes() {
        let sensor = Sensor::new(
            "s",
            "argo",
            "default",
            SensorSpec {
                dependencies: vec![EventDependency {
                    name: "ci".into(),
                    event_source_name: "github".into(),
                    event_name: "push".into(),
                    filters: Filters {
                        context: Some(ContextFilter {
                            r#type: Some("push".into()),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                }],
                triggers: vec![Trigger {
                    name: "deploy".into(),
                    template: TriggerTemplate::ArgoWorkflow {
                        operation: "submit".into(),
                        source: serde_json::json!({}),
                    },
                    retry_strategy: None,
                }],
                error_on_failed_round: false,
            },
        );
        let fired = evaluate_sensor(&sensor, &ev("push", serde_json::json!({})), "ci");
        assert_eq!(fired.len(), 1);
        let none = evaluate_sensor(&sensor, &ev("nope", serde_json::json!({})), "ci");
        assert!(none.is_empty());
    }

    #[test]
    fn sensor_with_unknown_dep_returns_empty() {
        let sensor = Sensor::new(
            "s",
            "argo",
            "default",
            SensorSpec {
                dependencies: vec![],
                triggers: vec![],
                error_on_failed_round: false,
            },
        );
        let fired = evaluate_sensor(&sensor, &ev("p", serde_json::json!({})), "ghost");
        assert!(fired.is_empty());
    }

    #[test]
    fn all_six_trigger_templates_construct_and_roundtrip() {
        for t in [
            TriggerTemplate::K8s {
                group: "".into(),
                version: "v1".into(),
                resource: "configmaps".into(),
                operation: "create".into(),
                source: serde_json::json!({}),
            },
            TriggerTemplate::Http {
                url: "https://x".into(),
                method: "POST".into(),
                payload: serde_json::json!({}),
            },
            TriggerTemplate::ArgoWorkflow {
                operation: "submit".into(),
                source: serde_json::json!({}),
            },
            TriggerTemplate::Kafka {
                url: "k".into(),
                topic: "t".into(),
                partition: None,
                payload: serde_json::json!({}),
            },
            TriggerTemplate::Slack {
                channel: "#deploys".into(),
                message: "deployed".into(),
                webhook_url_secret_ref: "kv/slack".into(),
            },
            TriggerTemplate::Pulsar {
                url: "p".into(),
                topic: "t".into(),
                payload: serde_json::json!({}),
            },
        ] {
            let j = serde_json::to_string(&t).unwrap();
            let _back: TriggerTemplate = serde_json::from_str(&j).unwrap();
        }
    }

    #[test]
    fn event_bus_backends_construct() {
        let js = EventBus::new(
            "default",
            "argo-events",
            EventBusBackend::JetStream {
                version: "2.10.10".into(),
                replicas: 3,
            },
        );
        assert!(matches!(js.backend, EventBusBackend::JetStream { .. }));
        let kf = EventBus::new(
            "kf",
            "argo-events",
            EventBusBackend::Kafka {
                brokers: vec!["b:9092".into()],
                topic_prefix: "argo".into(),
            },
        );
        assert!(matches!(kf.backend, EventBusBackend::Kafka { .. }));
    }

    #[test]
    fn data_filter_bool_equality() {
        let f = Filters {
            data: vec![DataFilter {
                path: "/done".into(),
                data_type: "bool".into(),
                value: vec!["true".into()],
                comparator: None,
            }],
            ..Default::default()
        };
        assert!(matches_filters(
            &ev("e", serde_json::json!({"done": true})),
            &f,
        ));
        assert!(!matches_filters(
            &ev("e", serde_json::json!({"done": false})),
            &f,
        ));
    }

    #[test]
    fn data_filter_or_semantics_across_value_list() {
        let f = Filters {
            data: vec![DataFilter {
                path: "/env".into(),
                data_type: "string".into(),
                value: vec!["prod".into(), "staging".into()],
                comparator: None,
            }],
            ..Default::default()
        };
        assert!(matches_filters(
            &ev("e", serde_json::json!({"env": "staging"})),
            &f,
        ));
        assert!(!matches_filters(
            &ev("e", serde_json::json!({"env": "dev"})),
            &f,
        ));
    }
}

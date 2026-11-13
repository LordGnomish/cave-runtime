// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! CloudEvents emission — KEDA's operator-side event emitter.
//! upstream: kedacore/keda v2.16.1
//!   pkg/eventemitter/cloudevent_http_handler.go (EmitEvent)
//!   pkg/eventemitter/eventhelper.go             (source/subject format)
//!   pkg/eventemitter/eventdata/eventdata.go     (EventData)
//!   apis/eventing/v1alpha1/cloudevent_types.go  (CloudEventType)
//!
//! The cloudevents-go SDK fills the envelope's `id` (a random UUID) and the
//! transport at send time; lacking that runtime here we derive a stable
//! content id (FNV-1a) so the same logical event reproduces the same id —
//! everything else (source/subject/type/data/specversion) is line-for-line.

use chrono::{DateTime, SecondsFormat, Utc};
use serde::Serialize;

/// CloudEvents structured-content spec version emitted by KEDA.
pub const CLOUDEVENTS_SPEC_VERSION: &str = "1.0";
/// `cloudevents.ApplicationJSON` — the datacontenttype KEDA sets.
pub const CLOUDEVENTS_DATACONTENTTYPE: &str = "application/json";

/// Port of `apis/eventing/v1alpha1/cloudevent_types.go` CloudEventType consts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloudEventType {
    ScaledObjectReady,
    ScaledObjectFailed,
    ScaledObjectRemoved,
    ScaledJobReady,
    ScaledJobFailed,
    ScaledJobRemoved,
    TriggerAuthenticationCreated,
    TriggerAuthenticationUpdated,
    TriggerAuthenticationRemoved,
    ClusterTriggerAuthenticationCreated,
    ClusterTriggerAuthenticationUpdated,
    ClusterTriggerAuthenticationRemoved,
}

impl CloudEventType {
    pub fn as_str(self) -> &'static str {
        match self {
            CloudEventType::ScaledObjectReady => "keda.scaledobject.ready.v1",
            CloudEventType::ScaledObjectFailed => "keda.scaledobject.failed.v1",
            CloudEventType::ScaledObjectRemoved => "keda.scaledobject.removed.v1",
            CloudEventType::ScaledJobReady => "keda.scaledjob.ready.v1",
            CloudEventType::ScaledJobFailed => "keda.scaledjob.failed.v1",
            CloudEventType::ScaledJobRemoved => "keda.scaledjob.removed.v1",
            CloudEventType::TriggerAuthenticationCreated => {
                "keda.authentication.triggerauthentication.created.v1"
            }
            CloudEventType::TriggerAuthenticationUpdated => {
                "keda.authentication.triggerauthentication.updated.v1"
            }
            CloudEventType::TriggerAuthenticationRemoved => {
                "keda.authentication.triggerauthentication.removed.v1"
            }
            CloudEventType::ClusterTriggerAuthenticationCreated => {
                "keda.authentication.clustertriggerauthentication.created.v1"
            }
            CloudEventType::ClusterTriggerAuthenticationUpdated => {
                "keda.authentication.clustertriggerauthentication.updated.v1"
            }
            CloudEventType::ClusterTriggerAuthenticationRemoved => {
                "keda.authentication.clustertriggerauthentication.removed.v1"
            }
        }
    }
}

/// Port of `eventdata.EventData` (the fields the emitter reads).
#[derive(Debug, Clone)]
pub struct EventData {
    pub namespace: String,
    pub object_name: String,
    pub object_type: String,
    pub cloud_event_type: CloudEventType,
    pub reason: String,
    pub message: String,
    pub time: DateTime<Utc>,
}

/// Port of `EmitData` — the JSON body of a KEDA CloudEvent.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct EmitData {
    pub reason: String,
    pub message: String,
}

/// `generateCloudEventSource` — `/<clusterName>/<kedaNamespace>/keda`.
pub fn generate_cloud_event_source(cluster_name: &str, keda_namespace: &str) -> String {
    format!("/{cluster_name}/{keda_namespace}/keda")
}

/// `generateCloudEventSubject` — `/<cluster>/<namespace>/<objectType>/<objectName>`.
pub fn generate_cloud_event_subject(
    cluster_name: &str,
    object_namespace: &str,
    object_type: &str,
    object_name: &str,
) -> String {
    format!("/{cluster_name}/{object_namespace}/{object_type}/{object_name}")
}

/// A fully-built CloudEvent in structured-content shape.
#[derive(Debug, Clone, Serialize)]
pub struct CloudEvent {
    pub specversion: String,
    pub id: String,
    pub source: String,
    #[serde(rename = "type")]
    pub ty: String,
    pub subject: String,
    pub datacontenttype: String,
    pub time: String,
    pub data: EmitData,
}

impl CloudEvent {
    /// Serialize to the CloudEvents structured-content JSON KEDA publishes.
    pub fn to_structured_json(&self) -> String {
        serde_json::to_string(self).expect("CloudEvent is always serializable")
    }
}

/// The operator-side emitter — holds cluster identity like KEDA's EventEmitter.
#[derive(Debug, Clone)]
pub struct CloudEventEmitter {
    pub cluster_name: String,
    /// `util.GetClusterObjectNamespace()` — the operator namespace (default `keda`).
    pub keda_namespace: String,
}

impl CloudEventEmitter {
    pub fn new(cluster_name: &str) -> Self {
        Self {
            cluster_name: cluster_name.to_string(),
            keda_namespace: "keda".to_string(),
        }
    }

    /// Port of `EmitEvent`: build the CloudEvent envelope from EventData.
    pub fn emit(&self, event: &EventData) -> CloudEvent {
        let source = generate_cloud_event_source(&self.cluster_name, &self.keda_namespace);
        let subject = generate_cloud_event_subject(
            &self.cluster_name,
            &event.namespace,
            &event.object_type,
            &event.object_name,
        );
        let ty = event.cloud_event_type.as_str().to_string();
        let data = EmitData {
            reason: event.reason.clone(),
            message: event.message.clone(),
        };
        let time = event.time.to_rfc3339_opts(SecondsFormat::Secs, true);
        let id = content_id(&source, &subject, &ty, &data, &time);
        CloudEvent {
            specversion: CLOUDEVENTS_SPEC_VERSION.to_string(),
            id,
            source,
            ty,
            subject,
            datacontenttype: CLOUDEVENTS_DATACONTENTTYPE.to_string(),
            time,
            data,
        }
    }
}

/// Stable FNV-1a-64 content id (cloudevents-go would mint a random UUID; we
/// keep a deterministic id so identical events reproduce identically).
fn content_id(source: &str, subject: &str, ty: &str, data: &EmitData, time: &str) -> String {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut h = OFFSET;
    let mut feed = |s: &str| {
        for b in s.bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(PRIME);
        }
        h ^= 0x1f; // field separator
        h = h.wrapping_mul(PRIME);
    };
    feed(source);
    feed(subject);
    feed(ty);
    feed(&data.reason);
    feed(&data.message);
    feed(time);
    format!("{h:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(secs, 0).single().unwrap()
    }

    fn sample() -> EventData {
        EventData {
            namespace: "default".into(),
            object_name: "my-so".into(),
            object_type: "ScaledObject".into(),
            cloud_event_type: CloudEventType::ScaledObjectReady,
            reason: "KEDAScalersStarted".into(),
            message: "Started scalers watch".into(),
            time: at(1_700_000_000),
        }
    }

    #[test]
    fn source_format_matches_upstream() {
        assert_eq!(
            generate_cloud_event_source("kind-kind", "keda"),
            "/kind-kind/keda/keda"
        );
    }

    #[test]
    fn subject_format_matches_upstream() {
        assert_eq!(
            generate_cloud_event_subject("kind-kind", "default", "ScaledObject", "my-so"),
            "/kind-kind/default/ScaledObject/my-so"
        );
    }

    #[test]
    fn cloud_event_type_strings_are_versioned() {
        assert_eq!(
            CloudEventType::ScaledObjectReady.as_str(),
            "keda.scaledobject.ready.v1"
        );
        assert_eq!(
            CloudEventType::ScaledJobFailed.as_str(),
            "keda.scaledjob.failed.v1"
        );
        assert_eq!(
            CloudEventType::ClusterTriggerAuthenticationRemoved.as_str(),
            "keda.authentication.clustertriggerauthentication.removed.v1"
        );
    }

    #[test]
    fn emit_builds_envelope_fields() {
        let e = CloudEventEmitter::new("kind-kind");
        let ce = e.emit(&sample());
        assert_eq!(ce.specversion, "1.0");
        assert_eq!(ce.source, "/kind-kind/keda/keda");
        assert_eq!(ce.subject, "/kind-kind/default/ScaledObject/my-so");
        assert_eq!(ce.ty, "keda.scaledobject.ready.v1");
        assert_eq!(ce.datacontenttype, "application/json");
        assert_eq!(ce.data.reason, "KEDAScalersStarted");
        assert_eq!(ce.data.message, "Started scalers watch");
    }

    #[test]
    fn structured_json_carries_renamed_type_and_data() {
        let e = CloudEventEmitter::new("kind-kind");
        let json = e.emit(&sample()).to_structured_json();
        assert!(json.contains("\"specversion\":\"1.0\""));
        assert!(json.contains("\"type\":\"keda.scaledobject.ready.v1\""));
        assert!(json.contains("\"data\":{\"reason\":\"KEDAScalersStarted\""));
        // the Go field is `type`, never `ty`
        assert!(!json.contains("\"ty\""));
    }

    #[test]
    fn content_id_is_stable_and_input_sensitive() {
        let e = CloudEventEmitter::new("kind-kind");
        let id1 = e.emit(&sample()).id;
        let id2 = e.emit(&sample()).id;
        assert_eq!(id1, id2, "same event → same id");

        let mut other = sample();
        other.message = "different".into();
        let id3 = e.emit(&other).id;
        assert_ne!(id1, id3, "different message → different id");
    }

    #[test]
    fn emit_respects_event_namespace_and_type_in_subject() {
        let e = CloudEventEmitter::new("prod");
        let mut ev = sample();
        ev.namespace = "team-a".into();
        ev.object_type = "ScaledJob".into();
        ev.object_name = "batch".into();
        ev.cloud_event_type = CloudEventType::ScaledJobRemoved;
        let ce = e.emit(&ev);
        assert_eq!(ce.subject, "/prod/team-a/ScaledJob/batch");
        assert_eq!(ce.ty, "keda.scaledjob.removed.v1");
    }
}

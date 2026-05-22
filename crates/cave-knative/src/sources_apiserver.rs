// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! ApiServerSource — emit CloudEvents on Kubernetes API resource changes.
//!
//! upstream: knative/eventing — pkg/apis/sources/v1/apiserver_types.go
//! + pkg/adapter/v2/apiserver/adapter.go
//!
//! The upstream adapter watches an informer for a set of GVRs and re-shapes
//! ADD / UPDATE / DELETE events into CloudEvents. We reproduce the same
//! envelope shape and resource-filter semantics; the actual watch is owned
//! by cave-apiserver. Here we keep the resource list, the label/expression
//! match, the optional resource-owner constraint, and the event mode
//! (`Reference` vs `Resource`).

use crate::meta::ObjectMeta;
use crate::sources_ping::CloudEvent;
use std::collections::HashMap;

#[derive(Default, Debug, Clone)]
pub struct ApiServerSource {
    pub metadata: ObjectMeta,
    pub spec: ApiServerSourceSpec,
    pub status: ApiServerSourceStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventMode {
    /// CloudEvent `data` is an ObjectReference (`apiVersion`+`kind`+`name`).
    Reference,
    /// CloudEvent `data` is the full resource as JSON.
    Resource,
}

impl Default for EventMode {
    fn default() -> Self {
        EventMode::Reference
    }
}

#[derive(Default, Debug, Clone)]
pub struct ApiServerSourceSpec {
    /// (api_version, kind) — e.g. ("v1", "Pod") or ("apps/v1", "Deployment").
    pub resources: Vec<(String, String)>,
    /// Optional label selector — every key/value must match the resource.
    pub label_selector: HashMap<String, String>,
    /// Optional owner kind constraint (e.g. "Job") — when set, only events
    /// whose `owner_kind` matches are emitted.
    pub owner_kind: Option<String>,
    /// Reference or full resource.
    pub mode: EventMode,
    /// Destination URL.
    pub sink: Option<String>,
    /// Optional service-account name used by the upstream RBAC binding.
    pub service_account_name: Option<String>,
}

#[derive(Default, Debug, Clone)]
pub struct ApiServerSourceStatus {
    pub sink_uri: Option<String>,
    pub observed_generation: i64,
}

/// One raw informer event the source receives.
#[derive(Debug, Clone)]
pub struct ResourceEvent {
    pub event_type: ResourceEventType,
    pub api_version: String,
    pub kind: String,
    pub namespace: String,
    pub name: String,
    pub uid: String,
    pub labels: HashMap<String, String>,
    pub owner_kind: Option<String>,
    pub resource_json: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceEventType {
    Add,
    Update,
    Delete,
}

impl ResourceEventType {
    pub fn ce_type(&self) -> &'static str {
        match self {
            ResourceEventType::Add => "dev.knative.apiserver.resource.add",
            ResourceEventType::Update => "dev.knative.apiserver.resource.update",
            ResourceEventType::Delete => "dev.knative.apiserver.resource.delete",
        }
    }
}

impl ApiServerSource {
    pub fn new(tenant_id: &str) -> Self {
        Self {
            metadata: ObjectMeta::with_creator(tenant_id),
            spec: ApiServerSourceSpec::default(),
            status: ApiServerSourceStatus::default(),
        }
    }

    pub fn with_resource(mut self, api_version: &str, kind: &str) -> Self {
        self.spec
            .resources
            .push((api_version.to_string(), kind.to_string()));
        self
    }

    pub fn resolve_sink(&mut self) -> Option<&str> {
        if let Some(ref s) = self.spec.sink {
            self.status.sink_uri = Some(s.clone());
        }
        self.status.sink_uri.as_deref()
    }

    /// Does the raw event pass the source's filters?
    pub fn matches(&self, ev: &ResourceEvent) -> bool {
        let pair = (ev.api_version.clone(), ev.kind.clone());
        if !self.spec.resources.is_empty() && !self.spec.resources.contains(&pair) {
            return false;
        }
        if let Some(ref ok) = self.spec.owner_kind {
            if ev.owner_kind.as_deref() != Some(ok.as_str()) {
                return false;
            }
        }
        for (k, want) in &self.spec.label_selector {
            if ev.labels.get(k).map(|s| s.as_str()) != Some(want.as_str()) {
                return false;
            }
        }
        true
    }

    /// Shape a single raw event into a CloudEvent envelope, respecting
    /// EventMode (reference / full resource).
    pub fn emit(&self, ev: &ResourceEvent, event_id: &str) -> Option<CloudEvent> {
        if !self.matches(ev) {
            return None;
        }
        let data = match self.spec.mode {
            EventMode::Reference => Some(format!(
                "{{\"apiVersion\":\"{}\",\"kind\":\"{}\",\"namespace\":\"{}\",\"name\":\"{}\",\"uid\":\"{}\"}}",
                ev.api_version, ev.kind, ev.namespace, ev.name, ev.uid
            )),
            EventMode::Resource => ev.resource_json.clone(),
        };
        let mut extensions: HashMap<String, String> = HashMap::new();
        extensions.insert("knsourceuid".to_string(), ev.uid.clone());
        extensions.insert("knnamespace".to_string(), ev.namespace.clone());
        Some(CloudEvent {
            id: event_id.to_string(),
            source: format!(
                "/apis/sources.knative.dev/v1/namespaces/{}/apiserversources/{}",
                self.metadata.namespace, self.metadata.name
            ),
            spec_version: "1.0".to_string(),
            event_type: ev.event_type.ce_type().to_string(),
            content_type: "application/json".to_string(),
            data,
            extensions,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(kind: &str, name: &str) -> ResourceEvent {
        ResourceEvent {
            event_type: ResourceEventType::Add,
            api_version: "v1".to_string(),
            kind: kind.to_string(),
            namespace: "default".to_string(),
            name: name.to_string(),
            uid: "uid-1".to_string(),
            labels: HashMap::new(),
            owner_kind: None,
            resource_json: Some("{\"hello\":1}".to_string()),
        }
    }

    #[test]
    fn matches_when_resource_listed() {
        let src = ApiServerSource::new("t").with_resource("v1", "Pod");
        assert!(src.matches(&raw("Pod", "p1")));
        assert!(!src.matches(&raw("Service", "s1")));
    }

    #[test]
    fn matches_empty_resource_list_is_wildcard() {
        let src = ApiServerSource::new("t");
        assert!(src.matches(&raw("Pod", "p1")));
        assert!(src.matches(&raw("ConfigMap", "c1")));
    }

    #[test]
    fn matches_owner_kind_constraint() {
        let mut src = ApiServerSource::new("t").with_resource("v1", "Pod");
        src.spec.owner_kind = Some("Job".to_string());
        let mut r = raw("Pod", "p1");
        r.owner_kind = Some("Job".to_string());
        assert!(src.matches(&r));
        let mut r2 = raw("Pod", "p2");
        r2.owner_kind = Some("ReplicaSet".to_string());
        assert!(!src.matches(&r2));
    }

    #[test]
    fn matches_label_selector_all_required() {
        let mut src = ApiServerSource::new("t").with_resource("v1", "Pod");
        src.spec
            .label_selector
            .insert("app".to_string(), "demo".to_string());
        let mut r = raw("Pod", "p1");
        r.labels.insert("app".to_string(), "demo".to_string());
        assert!(src.matches(&r));
        let mut r2 = raw("Pod", "p2");
        r2.labels.insert("app".to_string(), "other".to_string());
        assert!(!src.matches(&r2));
    }

    #[test]
    fn emit_reference_mode_returns_object_ref_json() {
        let src = ApiServerSource::new("t").with_resource("v1", "Pod");
        let ev = src.emit(&raw("Pod", "p1"), "id-1").unwrap();
        assert_eq!(ev.event_type, "dev.knative.apiserver.resource.add");
        let data = ev.data.as_deref().unwrap();
        assert!(data.contains("\"kind\":\"Pod\""));
        assert!(data.contains("\"name\":\"p1\""));
        assert!(data.contains("\"uid\":\"uid-1\""));
    }

    #[test]
    fn emit_resource_mode_returns_full_resource_json() {
        let mut src = ApiServerSource::new("t").with_resource("v1", "Pod");
        src.spec.mode = EventMode::Resource;
        let ev = src.emit(&raw("Pod", "p1"), "id-1").unwrap();
        assert_eq!(ev.data.as_deref(), Some("{\"hello\":1}"));
    }

    #[test]
    fn emit_update_and_delete_event_types() {
        let src = ApiServerSource::new("t").with_resource("v1", "Pod");
        let mut r = raw("Pod", "p1");
        r.event_type = ResourceEventType::Update;
        assert_eq!(
            src.emit(&r, "id-1").unwrap().event_type,
            "dev.knative.apiserver.resource.update"
        );
        r.event_type = ResourceEventType::Delete;
        assert_eq!(
            src.emit(&r, "id-1").unwrap().event_type,
            "dev.knative.apiserver.resource.delete"
        );
    }

    #[test]
    fn emit_returns_none_when_filter_fails() {
        let src = ApiServerSource::new("t").with_resource("v1", "Pod");
        assert!(src.emit(&raw("Service", "s1"), "id-1").is_none());
    }

    #[test]
    fn emit_extension_attributes_carry_uid_and_namespace() {
        let src = ApiServerSource::new("t").with_resource("v1", "Pod");
        let ev = src.emit(&raw("Pod", "p1"), "id-1").unwrap();
        assert_eq!(
            ev.extensions.get("knsourceuid").map(|s| s.as_str()),
            Some("uid-1")
        );
        assert_eq!(
            ev.extensions.get("knnamespace").map(|s| s.as_str()),
            Some("default")
        );
    }
}

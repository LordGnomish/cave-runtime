// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! k8s_audit Falco plugin — ingest k8s audit events and project into
//! `FalcoEvent` form so engine rules with `source: k8s_audit` can fire.
//!
//! NOTICE: upstream is falcosecurity/plugins/plugins/k8saudit (Apache-2.0).
//! Plugin discovery + dlopen are out-of-process (`falco_plugin_manager.cpp`).

use crate::error::{FalcoError, Result};
use crate::event::FalcoEvent;
use serde::{Deserialize, Serialize};

/// Subset of K8s API server audit event JSON (`audit.k8s.io/v1`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditEvent {
    #[serde(default)]
    pub kind: String,
    #[serde(rename = "apiVersion", default)]
    pub api_version: String,
    pub level: String,
    pub stage: String,
    #[serde(default)]
    pub verb: String,
    pub user: Option<AuditUser>,
    #[serde(rename = "requestURI", default)]
    pub request_uri: String,
    #[serde(rename = "objectRef")]
    pub object_ref: Option<ObjectRef>,
    #[serde(rename = "responseStatus")]
    pub response_status: Option<ResponseStatus>,
    #[serde(rename = "stageTimestamp", default)]
    pub stage_timestamp: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditUser {
    pub username: String,
    #[serde(default)]
    pub groups: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectRef {
    pub resource: String,
    #[serde(default)]
    pub namespace: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub subresource: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResponseStatus {
    #[serde(default)]
    pub code: i32,
    #[serde(default)]
    pub status: String,
}

/// Project a K8s audit event into a FalcoEvent that the engine's
/// `ka.*` field references resolve against.
pub fn project(audit: &AuditEvent) -> FalcoEvent {
    let mut ev = FalcoEvent::k8s_audit(&audit.stage, &audit.verb);
    if let Some(u) = &audit.user {
        ev.fields.insert("ka.user.name".into(), u.username.clone());
        ev.fields.insert("ka.user.groups".into(), u.groups.join(","));
    }
    if let Some(o) = &audit.object_ref {
        ev.fields.insert("ka.target.resource".into(), o.resource.clone());
        ev.fields.insert("ka.target.namespace".into(), o.namespace.clone());
        ev.fields.insert("ka.target.name".into(), o.name.clone());
        if !o.subresource.is_empty() {
            ev.fields.insert("ka.target.subresource".into(), o.subresource.clone());
        }
    }
    if let Some(r) = &audit.response_status {
        ev.fields.insert("ka.response.code".into(), r.code.to_string());
        ev.fields.insert("ka.response.status".into(), r.status.clone());
    }
    ev.fields.insert("ka.uri".into(), audit.request_uri.clone());
    ev
}

pub fn parse(json: &str) -> Result<AuditEvent> {
    serde_json::from_str(json).map_err(FalcoError::Json)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> AuditEvent {
        AuditEvent {
            kind: "Event".into(),
            api_version: "audit.k8s.io/v1".into(),
            level: "RequestResponse".into(),
            stage: "ResponseComplete".into(),
            verb: "create".into(),
            user: Some(AuditUser { username: "system:serviceaccount:dev:builder".into(), groups: vec!["system:authenticated".into()] }),
            request_uri: "/api/v1/namespaces/dev/pods".into(),
            object_ref: Some(ObjectRef { resource: "pods".into(), namespace: "dev".into(), name: "app-1".into(), subresource: String::new() }),
            response_status: Some(ResponseStatus { code: 201, status: "Created".into() }),
            stage_timestamp: "2026-05-24T10:00:00Z".into(),
        }
    }

    #[test]
    fn projects_user_resource_namespace_into_ka_fields() {
        let ev = project(&sample());
        assert_eq!(ev.fields.get("ka.verb").unwrap(), "create");
        assert_eq!(ev.fields.get("ka.user.name").unwrap(), "system:serviceaccount:dev:builder");
        assert_eq!(ev.fields.get("ka.target.resource").unwrap(), "pods");
        assert_eq!(ev.fields.get("ka.target.namespace").unwrap(), "dev");
        assert_eq!(ev.fields.get("ka.response.code").unwrap(), "201");
    }

    #[test]
    fn projects_subresource_when_present() {
        let mut s = sample();
        s.object_ref.as_mut().unwrap().subresource = "exec".into();
        let ev = project(&s);
        assert_eq!(ev.fields.get("ka.target.subresource").unwrap(), "exec");
    }

    #[test]
    fn projects_groups_joined_with_comma() {
        let ev = project(&sample());
        assert_eq!(ev.fields.get("ka.user.groups").unwrap(), "system:authenticated");
    }

    #[test]
    fn parses_minimal_audit_json() {
        let j = r#"{"level":"Metadata","stage":"RequestReceived"}"#;
        let a = parse(j).unwrap();
        assert_eq!(a.stage, "RequestReceived");
    }

    #[test]
    fn parse_invalid_json_errors() {
        let r = parse("{not-json");
        assert!(r.is_err());
    }

    #[test]
    fn projection_class_is_k8s_audit() {
        let ev = project(&sample());
        assert_eq!(ev.class, crate::event::EventClass::K8sAudit);
        assert_eq!(ev.source, "k8s_audit");
    }
}

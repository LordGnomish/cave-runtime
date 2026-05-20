// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! ContainerSource — run a container that pushes CloudEvents to a sink.
//!
//! upstream: knative/eventing — pkg/apis/sources/v1/container_types.go
//!
//! In the upstream, the reconciler creates a Deployment that runs the
//! given container with `K_SINK` and `K_CE_OVERRIDES` env injected. We
//! port the controller-side projection: given a ContainerSourceSpec, we
//! produce the `(name, image, env)` triple that the cave-runtime
//! controller will hand to cave-keda/cave-controller-manager for
//! creation. No actual pod is launched here — that is the job of the
//! data-plane components — but the env injection is exhaustive.

use std::collections::HashMap;
use crate::meta::ObjectMeta;
use crate::meta::{Container, EnvVar, PodSpec};

#[derive(Default, Debug, Clone)]
pub struct ContainerSource {
    pub metadata: ObjectMeta,
    pub spec: ContainerSourceSpec,
    pub status: ContainerSourceStatus,
}

#[derive(Default, Debug, Clone)]
pub struct ContainerSourceSpec {
    pub template: PodSpec,
    pub sink: Option<String>,
    pub ce_overrides: HashMap<String, String>,
}

#[derive(Default, Debug, Clone)]
pub struct ContainerSourceStatus {
    pub sink_uri: Option<String>,
    pub deployment_name: Option<String>,
    pub observed_generation: i64,
}

impl ContainerSource {
    pub fn new(tenant_id: &str, name: &str) -> Self {
        let mut s = Self {
            metadata: ObjectMeta::with_creator(tenant_id),
            spec: ContainerSourceSpec::default(),
            status: ContainerSourceStatus::default(),
        };
        s.metadata.name = name.to_string();
        s
    }

    pub fn resolve_sink(&mut self) -> Option<&str> {
        if let Some(ref s) = self.spec.sink {
            self.status.sink_uri = Some(s.clone());
        }
        self.status.sink_uri.as_deref()
    }

    /// Project the source into the PodSpec the Deployment will run.
    /// Injects `K_SINK` (the resolved sink) and `K_CE_OVERRIDES` (a JSON
    /// object built from `spec.ce_overrides`) onto every container.
    pub fn project(&self) -> PodSpec {
        let sink = self.spec.sink.clone().unwrap_or_default();
        let ce_overrides_json = ce_overrides_to_json(&self.spec.ce_overrides);
        let mut pod = self.spec.template.clone();
        for c in pod.containers.iter_mut() {
            inject_or_replace_env(c, "K_SINK", &sink);
            inject_or_replace_env(c, "K_CE_OVERRIDES", &ce_overrides_json);
            inject_or_replace_env(c, "K_NAME", &self.metadata.name);
            inject_or_replace_env(c, "K_NAMESPACE", &self.metadata.namespace);
        }
        pod
    }

    /// Name used for the spawned Deployment (matches upstream's "<name>"
    /// scheme for ContainerSource owner refs).
    pub fn deployment_name(&self) -> String {
        format!("{}-containersource", self.metadata.name)
    }
}

fn inject_or_replace_env(c: &mut Container, key: &str, value: &str) {
    for e in c.env.iter_mut() {
        if e.name == key {
            e.value = Some(value.to_string());
            return;
        }
    }
    c.env.push(EnvVar { name: key.to_string(), value: Some(value.to_string()) });
}

/// Encode `ce_overrides` as the JSON object the upstream Go adapter expects:
///   {"extensions":{"k1":"v1","k2":"v2"}}
pub fn ce_overrides_to_json(map: &HashMap<String, String>) -> String {
    let mut entries: Vec<(&String, &String)> = map.iter().collect();
    entries.sort_by(|a, b| a.0.cmp(b.0));
    let body: String = entries
        .iter()
        .map(|(k, v)| format!("\"{}\":\"{}\"", escape_json(k), escape_json(v)))
        .collect::<Vec<_>>()
        .join(",");
    format!("{{\"extensions\":{{{}}}}}", body)
}

fn escape_json(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cs(name: &str, image: &str) -> ContainerSource {
        let mut s = ContainerSource::new("t", name);
        s.metadata.namespace = "default".to_string();
        s.spec.template = PodSpec {
            containers: vec![Container {
                name: "app".to_string(),
                image: image.to_string(),
                env: vec![],
            }],
        };
        s.spec.sink = Some("https://sink.example/in".to_string());
        s
    }

    #[test]
    fn project_injects_k_sink() {
        let s = cs("watcher", "emitter:1");
        let pod = s.project();
        let k_sink = pod.containers[0]
            .env
            .iter()
            .find(|e| e.name == "K_SINK")
            .and_then(|e| e.value.clone());
        assert_eq!(k_sink.as_deref(), Some("https://sink.example/in"));
    }

    #[test]
    fn project_injects_k_ce_overrides_as_json_object() {
        let mut s = cs("watcher", "emitter:1");
        s.spec.ce_overrides.insert("tenant".to_string(), "alpha".to_string());
        s.spec.ce_overrides.insert("region".to_string(), "eu-1".to_string());
        let pod = s.project();
        let json = pod.containers[0]
            .env
            .iter()
            .find(|e| e.name == "K_CE_OVERRIDES")
            .and_then(|e| e.value.clone())
            .unwrap();
        // sorted keys: region, tenant
        assert_eq!(
            json,
            "{\"extensions\":{\"region\":\"eu-1\",\"tenant\":\"alpha\"}}"
        );
    }

    #[test]
    fn project_replaces_existing_env_rather_than_appending() {
        let mut s = cs("w", "img:1");
        s.spec.template.containers[0].env.push(EnvVar {
            name: "K_SINK".to_string(),
            value: Some("stale".to_string()),
        });
        let pod = s.project();
        let k_sink: Vec<&EnvVar> = pod.containers[0].env.iter().filter(|e| e.name == "K_SINK").collect();
        assert_eq!(k_sink.len(), 1);
        assert_eq!(k_sink[0].value.as_deref(), Some("https://sink.example/in"));
    }

    #[test]
    fn deployment_name_uses_source_name() {
        let s = cs("ticker", "img");
        assert_eq!(s.deployment_name(), "ticker-containersource");
    }

    #[test]
    fn k_namespace_carried_to_env() {
        let s = cs("w", "img:1");
        let pod = s.project();
        let ns = pod.containers[0]
            .env
            .iter()
            .find(|e| e.name == "K_NAMESPACE")
            .and_then(|e| e.value.clone());
        assert_eq!(ns.as_deref(), Some("default"));
    }

    #[test]
    fn ce_overrides_empty_emits_empty_object() {
        let s = cs("w", "img:1");
        let pod = s.project();
        let json = pod.containers[0]
            .env
            .iter()
            .find(|e| e.name == "K_CE_OVERRIDES")
            .and_then(|e| e.value.clone())
            .unwrap();
        assert_eq!(json, "{\"extensions\":{}}");
    }

    #[test]
    fn ce_overrides_escape_quotes() {
        let mut s = cs("w", "img:1");
        s.spec.ce_overrides.insert("k".to_string(), "\"v\"".to_string());
        let pod = s.project();
        let json = pod.containers[0]
            .env
            .iter()
            .find(|e| e.name == "K_CE_OVERRIDES")
            .and_then(|e| e.value.clone())
            .unwrap();
        assert_eq!(json, "{\"extensions\":{\"k\":\"\\\"v\\\"\"}}");
    }
}

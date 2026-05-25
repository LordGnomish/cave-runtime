// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! DeploymentRuntimeConfig (v2 successor to ControllerConfig) — pod template
//! overrides + service-account binding + env + replicas.
//!
//! Upstream: apis/pkg/v1beta1/deploymentruntimeconfig_types.go

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeploymentRuntimeConfig {
    pub name: String,
    pub service_account_name: Option<String>,
    pub replicas: u32,
    pub env: BTreeMap<String, String>,
    pub resources: Option<ResourceLimits>,
    pub node_selector: BTreeMap<String, String>,
    pub tolerations: Vec<Toleration>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResourceLimits {
    pub cpu: String,
    pub memory: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Toleration {
    pub key: String,
    pub operator: String,
    pub value: Option<String>,
    pub effect: Option<String>,
}

impl DeploymentRuntimeConfig {
    pub fn default_for(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            service_account_name: None,
            replicas: 1,
            env: BTreeMap::new(),
            resources: None,
            node_selector: BTreeMap::new(),
            tolerations: Vec::new(),
        }
    }

    pub fn with_env(mut self, k: impl Into<String>, v: impl Into<String>) -> Self {
        self.env.insert(k.into(), v.into());
        self
    }

    pub fn with_replicas(mut self, r: u32) -> Self {
        self.replicas = r;
        self
    }

    pub fn with_service_account(mut self, sa: impl Into<String>) -> Self {
        self.service_account_name = Some(sa.into());
        self
    }

    pub fn with_resources(mut self, cpu: impl Into<String>, memory: impl Into<String>) -> Self {
        self.resources = Some(ResourceLimits {
            cpu: cpu.into(),
            memory: memory.into(),
        });
        self
    }

    pub fn with_toleration(mut self, t: Toleration) -> Self {
        self.tolerations.push(t);
        self
    }

    pub fn with_node_selector(mut self, k: impl Into<String>, v: impl Into<String>) -> Self {
        self.node_selector.insert(k.into(), v.into());
        self
    }

    /// Render to a Kubernetes Deployment podSpec patch (minimal).
    pub fn to_pod_spec_patch(&self) -> serde_json::Value {
        let env: Vec<serde_json::Value> = self
            .env
            .iter()
            .map(|(k, v)| serde_json::json!({"name": k, "value": v}))
            .collect();
        let mut res = serde_json::json!({
            "replicas": self.replicas,
            "template": {
                "spec": {
                    "containers": [{
                        "name": "package-runtime",
                        "env": env,
                    }]
                }
            }
        });
        if let Some(sa) = &self.service_account_name {
            res["template"]["spec"]["serviceAccountName"] =
                serde_json::Value::String(sa.clone());
        }
        if let Some(r) = &self.resources {
            res["template"]["spec"]["containers"][0]["resources"] = serde_json::json!({
                "limits": {"cpu": r.cpu, "memory": r.memory},
            });
        }
        if !self.node_selector.is_empty() {
            let mut ns = serde_json::Map::new();
            for (k, v) in &self.node_selector {
                ns.insert(k.clone(), serde_json::Value::String(v.clone()));
            }
            res["template"]["spec"]["nodeSelector"] = serde_json::Value::Object(ns);
        }
        if !self.tolerations.is_empty() {
            res["template"]["spec"]["tolerations"] = serde_json::to_value(&self.tolerations).unwrap();
        }
        res
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_one_replica() {
        let d = DeploymentRuntimeConfig::default_for("provider-aws");
        assert_eq!(d.replicas, 1);
    }

    #[test]
    fn with_env_inserts() {
        let d = DeploymentRuntimeConfig::default_for("p").with_env("FOO", "bar");
        assert_eq!(d.env.get("FOO").unwrap(), "bar");
    }

    #[test]
    fn with_service_account_attaches() {
        let d = DeploymentRuntimeConfig::default_for("p").with_service_account("sa-1");
        assert_eq!(d.service_account_name.as_deref(), Some("sa-1"));
    }

    #[test]
    fn to_pod_spec_patch_shape() {
        let d = DeploymentRuntimeConfig::default_for("p")
            .with_replicas(3)
            .with_env("K", "V");
        let p = d.to_pod_spec_patch();
        assert_eq!(p["replicas"], serde_json::json!(3));
        assert_eq!(
            p["template"]["spec"]["containers"][0]["env"][0]["name"],
            serde_json::json!("K")
        );
    }

    #[test]
    fn pod_spec_resources_render() {
        let d = DeploymentRuntimeConfig::default_for("p").with_resources("500m", "256Mi");
        let p = d.to_pod_spec_patch();
        assert_eq!(
            p["template"]["spec"]["containers"][0]["resources"]["limits"]["cpu"],
            serde_json::json!("500m")
        );
    }

    #[test]
    fn pod_spec_node_selector_render() {
        let d = DeploymentRuntimeConfig::default_for("p").with_node_selector("zone", "eu-1");
        let p = d.to_pod_spec_patch();
        assert_eq!(
            p["template"]["spec"]["nodeSelector"]["zone"],
            serde_json::json!("eu-1")
        );
    }

    #[test]
    fn pod_spec_tolerations_render() {
        let d = DeploymentRuntimeConfig::default_for("p").with_toleration(Toleration {
            key: "dedicated".into(),
            operator: "Equal".into(),
            value: Some("crossplane".into()),
            effect: Some("NoSchedule".into()),
        });
        let p = d.to_pod_spec_patch();
        assert!(p["template"]["spec"]["tolerations"][0]["key"] == serde_json::json!("dedicated"));
    }

    #[test]
    fn round_trip_serde() {
        let d = DeploymentRuntimeConfig::default_for("p").with_replicas(2);
        let s = serde_json::to_string(&d).unwrap();
        let d2: DeploymentRuntimeConfig = serde_json::from_str(&s).unwrap();
        assert_eq!(d, d2);
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! TriggerAuthentication CRD — credentials for trigger sources.
//! upstream: kedacore/keda v2.x — apis/keda/v1alpha1/triggerauthentication_types.go

use std::collections::HashMap;

#[derive(Default, Debug, Clone)]
pub struct TriggerAuthentication {
    pub tenant_id: String,
    pub name: String,
    pub namespace: String,
    /// Secret references — `parameter -> secret_key`.
    pub secret_target_ref: Vec<SecretTargetRef>,
    /// EnvVar references — `parameter -> container_env_var_name`.
    pub env_target_ref: Vec<EnvTargetRef>,
    /// Inline parameters (cleartext) — for parity with hashiCorpVault and
    /// dependent parameter sourcing. Discouraged in upstream KEDA.
    pub inline: HashMap<String, String>,
    /// Resolved parameters after the controller fetches them from the upstream
    /// Secret/EnvVar/Vault sources. Empty until resolve() is called.
    pub resolved: HashMap<String, String>,
}

#[derive(Default, Debug, Clone)]
pub struct SecretTargetRef {
    pub parameter: String,
    pub name: String,
    pub key: String,
}

#[derive(Default, Debug, Clone)]
pub struct EnvTargetRef {
    pub parameter: String,
    pub name: String,
    pub container_name: Option<String>,
}

impl TriggerAuthentication {
    pub fn new(tenant_id: &str) -> Self {
        Self {
            tenant_id: tenant_id.to_string(),
            ..Default::default()
        }
    }

    /// Resolve parameters from a backing secret store. Real KEDA hits the
    /// Kubernetes API for Secrets / Pod EnvVars; this in-memory port takes
    /// the secret store as a HashMap of `{secret_name -> {key -> value}}`.
    pub fn resolve(
        &mut self,
        secret_store: &HashMap<String, HashMap<String, String>>,
        env_store: &HashMap<String, String>,
    ) {
        self.resolved.clear();

        for sref in &self.secret_target_ref {
            if let Some(map) = secret_store.get(&sref.name) {
                if let Some(v) = map.get(&sref.key) {
                    self.resolved.insert(sref.parameter.clone(), v.clone());
                }
            }
        }
        for eref in &self.env_target_ref {
            if let Some(v) = env_store.get(&eref.name) {
                self.resolved.insert(eref.parameter.clone(), v.clone());
            }
        }
        for (k, v) in &self.inline {
            self.resolved.entry(k.clone()).or_insert_with(|| v.clone());
        }
    }

    pub fn parameter(&self, name: &str) -> Option<&str> {
        self.resolved.get(name).map(String::as_str)
    }

    pub fn add_secret_ref(&mut self, parameter: &str, name: &str, key: &str) {
        self.secret_target_ref.push(SecretTargetRef {
            parameter: parameter.to_string(),
            name: name.to_string(),
            key: key.to_string(),
        });
    }
}

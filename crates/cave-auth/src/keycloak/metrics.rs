// SPDX-License-Identifier: AGPL-3.0-or-later
//! Prometheus instrumentation for the Keycloak OIDC/admin endpoint suite.
//!
//! Metrics:
//!   cave_auth_authorize_requests_total{realm,response_type,prompt}
//!   cave_auth_device_flow_total{realm,phase}
//!   cave_auth_ciba_total{realm,result}
//!   cave_auth_par_total{realm,result}
//!   cave_auth_revoke_total{realm,token_type_hint,result}
//!   cave_auth_idp_admin_operations_total{op,result}
//!   cave_auth_flow_admin_operations_total{op,result}

use once_cell::sync::Lazy;
use prometheus_client::{
    encoding::{text::encode, EncodeLabelSet},
    metrics::{counter::Counter, family::Family},
    registry::Registry,
};
use std::sync::Mutex;

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct AuthorizeLabels {
    pub realm: String,
    pub response_type: String,
    pub prompt: String,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct DeviceLabels {
    pub realm: String,
    /// One of `auth | poll | verify | complete`.
    pub phase: String,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct RealmResultLabels {
    pub realm: String,
    pub result: String,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct RevokeLabels {
    pub realm: String,
    pub token_type_hint: String,
    pub result: String,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct AdminOpLabels {
    pub op: String,
    pub result: String,
}

pub struct KeycloakMetrics {
    pub authorize_requests_total: Family<AuthorizeLabels, Counter>,
    pub device_flow_total: Family<DeviceLabels, Counter>,
    pub ciba_total: Family<RealmResultLabels, Counter>,
    pub par_total: Family<RealmResultLabels, Counter>,
    pub revoke_total: Family<RevokeLabels, Counter>,
    pub idp_admin_operations_total: Family<AdminOpLabels, Counter>,
    pub flow_admin_operations_total: Family<AdminOpLabels, Counter>,
    registry: Mutex<Registry>,
}

impl KeycloakMetrics {
    fn new() -> Self {
        let mut registry = Registry::default();
        let authorize_requests_total = Family::<AuthorizeLabels, Counter>::default();
        let device_flow_total = Family::<DeviceLabels, Counter>::default();
        let ciba_total = Family::<RealmResultLabels, Counter>::default();
        let par_total = Family::<RealmResultLabels, Counter>::default();
        let revoke_total = Family::<RevokeLabels, Counter>::default();
        let idp_admin_operations_total = Family::<AdminOpLabels, Counter>::default();
        let flow_admin_operations_total = Family::<AdminOpLabels, Counter>::default();

        registry.register(
            "cave_auth_authorize_requests",
            "OAuth/OIDC /auth endpoint requests",
            authorize_requests_total.clone(),
        );
        registry.register(
            "cave_auth_device_flow",
            "RFC 8628 device authorization events by phase",
            device_flow_total.clone(),
        );
        registry.register(
            "cave_auth_ciba",
            "OIDC CIBA backchannel-authentication events",
            ciba_total.clone(),
        );
        registry.register(
            "cave_auth_par",
            "RFC 9126 Pushed Authorization Request events",
            par_total.clone(),
        );
        registry.register(
            "cave_auth_revoke",
            "RFC 7009 token revocation events",
            revoke_total.clone(),
        );
        registry.register(
            "cave_auth_idp_admin_operations",
            "Identity-Provider REST admin operations",
            idp_admin_operations_total.clone(),
        );
        registry.register(
            "cave_auth_flow_admin_operations",
            "AuthenticationFlow REST admin operations",
            flow_admin_operations_total.clone(),
        );

        Self {
            authorize_requests_total,
            device_flow_total,
            ciba_total,
            par_total,
            revoke_total,
            idp_admin_operations_total,
            flow_admin_operations_total,
            registry: Mutex::new(registry),
        }
    }

    /// Encode the global registry as Prometheus text exposition.
    pub fn encode(&self) -> String {
        let r = self.registry.lock().unwrap();
        let mut buf = String::new();
        encode(&mut buf, &r).expect("registry encode");
        buf
    }
}

pub static METRICS: Lazy<KeycloakMetrics> = Lazy::new(KeycloakMetrics::new);

pub fn inc_authorize(realm: &str, response_type: &str, prompt: &str) {
    METRICS.authorize_requests_total.get_or_create(&AuthorizeLabels {
        realm: realm.to_string(),
        response_type: response_type.to_string(),
        prompt: prompt.to_string(),
    }).inc();
}

pub fn inc_device(realm: &str, phase: &str) {
    METRICS.device_flow_total.get_or_create(&DeviceLabels {
        realm: realm.to_string(),
        phase: phase.to_string(),
    }).inc();
}

pub fn inc_ciba(realm: &str, result: &str) {
    METRICS.ciba_total.get_or_create(&RealmResultLabels {
        realm: realm.to_string(),
        result: result.to_string(),
    }).inc();
}

pub fn inc_par(realm: &str, result: &str) {
    METRICS.par_total.get_or_create(&RealmResultLabels {
        realm: realm.to_string(),
        result: result.to_string(),
    }).inc();
}

pub fn inc_revoke(realm: &str, hint: &str, result: &str) {
    METRICS.revoke_total.get_or_create(&RevokeLabels {
        realm: realm.to_string(),
        token_type_hint: hint.to_string(),
        result: result.to_string(),
    }).inc();
}

pub fn inc_idp_op(op: &str, result: &str) {
    METRICS.idp_admin_operations_total.get_or_create(&AdminOpLabels {
        op: op.to_string(),
        result: result.to_string(),
    }).inc();
}

pub fn inc_flow_op(op: &str, result: &str) {
    METRICS.flow_admin_operations_total.get_or_create(&AdminOpLabels {
        op: op.to_string(),
        result: result.to_string(),
    }).inc();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_emits_metric_names() {
        inc_authorize("realm1", "code", "none");
        inc_device("realm1", "auth");
        inc_ciba("realm1", "ok");
        inc_par("realm1", "ok");
        inc_revoke("realm1", "access_token", "ok");
        inc_idp_op("create", "ok");
        inc_flow_op("create", "ok");
        let s = METRICS.encode();
        assert!(s.contains("cave_auth_authorize_requests_total"));
        assert!(s.contains("cave_auth_device_flow_total"));
        assert!(s.contains("cave_auth_ciba_total"));
        assert!(s.contains("cave_auth_par_total"));
        assert!(s.contains("cave_auth_revoke_total"));
        assert!(s.contains("cave_auth_idp_admin_operations_total"));
        assert!(s.contains("cave_auth_flow_admin_operations_total"));
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
//! GRPCRoute.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrpcMethodMatch {
    #[serde(default)] pub method: Option<String>,
    #[serde(default)] pub service: Option<String>,
    #[serde(default)] pub kind: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrpcRouteRule {
    #[serde(default)] pub matches: Vec<GrpcMethodMatch>,
    #[serde(default)] pub backend_refs: Vec<crate::crd::httproute::HttpBackend>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrpcRoute {
    pub name: String, pub parent_refs: Vec<String>,
    #[serde(default)] pub hostnames: Vec<String>,
    pub rules: Vec<GrpcRouteRule>,
}
impl GrpcRoute {
    pub fn new(name: &str) -> Self { Self { name: name.into(), parent_refs: vec![], hostnames: vec![], rules: vec![] } }
    pub fn validate(&self) -> Result<(), String> {
        if self.rules.is_empty() { return Err("GRPCRoute must have rules".into()); }
        for r in &self.rules { if r.backend_refs.is_empty() { return Err("rule must have backend_refs".into()); } }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crd::httproute::HttpBackend;
    #[test] fn empty_rules_rejected() { assert!(GrpcRoute::new("r").validate().is_err()); }
    #[test] fn rule_without_backend_rejected() {
        let mut r = GrpcRoute::new("r");
        r.rules.push(GrpcRouteRule { matches: vec![], backend_refs: vec![] });
        assert!(r.validate().is_err());
    }
    #[test] fn valid() {
        let mut r = GrpcRoute::new("r");
        r.rules.push(GrpcRouteRule {
            matches: vec![GrpcMethodMatch { method: Some("GetUser".into()), service: Some("users.v1.UsersService".into()), kind: Some("Exact".into()) }],
            backend_refs: vec![HttpBackend { name: "users-svc".into(), port: 50051, weight: 1 }],
        });
        r.validate().unwrap();
    }
    #[test] fn match_optional_fields() {
        let m = GrpcMethodMatch { method: None, service: None, kind: None };
        assert!(m.method.is_none());
    }
}

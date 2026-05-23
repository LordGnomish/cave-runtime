// SPDX-License-Identifier: AGPL-3.0-or-later
//! HTTPRoute.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpBackend {
    pub name: String, pub port: u16, #[serde(default = "default_weight")] pub weight: u32,
}
fn default_weight() -> u32 { 1 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpRouteRule {
    #[serde(default)] pub matches: Vec<HttpMatch>,
    #[serde(default)] pub backends: Vec<HttpBackend>,
    #[serde(default)] pub filters: Vec<HttpFilter>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpMatch {
    #[serde(default)] pub path: Option<PathMatch>,
    #[serde(default)] pub method: Option<String>,
    #[serde(default)] pub headers: Vec<HeaderMatch>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathMatch { pub kind: String, pub value: String }
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeaderMatch { pub name: String, pub value: String }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpFilter { pub kind: String, pub config: serde_json::Value }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpRoute {
    pub name: String, pub parent_refs: Vec<String>,
    #[serde(default)] pub hostnames: Vec<String>,
    pub rules: Vec<HttpRouteRule>,
}
impl HttpRoute {
    pub fn new(name: &str) -> Self { Self { name: name.into(), parent_refs: vec![], hostnames: vec![], rules: vec![] } }
    pub fn validate(&self) -> Result<(), String> {
        if self.rules.is_empty() { return Err("HTTPRoute must have rules".into()); }
        for r in &self.rules { if r.backends.is_empty() { return Err("rule must have backends".into()); } }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn empty_rules_rejected() { assert!(HttpRoute::new("r").validate().is_err()); }
    #[test] fn rule_without_backends_rejected() {
        let mut r = HttpRoute::new("r");
        r.rules.push(HttpRouteRule { matches: vec![], backends: vec![], filters: vec![] });
        assert!(r.validate().is_err());
    }
    #[test] fn valid_route() {
        let mut r = HttpRoute::new("r");
        r.rules.push(HttpRouteRule {
            matches: vec![HttpMatch { path: Some(PathMatch { kind: "PathPrefix".into(), value: "/api".into() }), method: None, headers: vec![] }],
            backends: vec![HttpBackend { name: "svc".into(), port: 80, weight: 1 }],
            filters: vec![],
        });
        r.validate().unwrap();
    }
    #[test] fn weight_default() {
        let b: HttpBackend = serde_json::from_str(r#"{"name":"a","port":80}"#).unwrap();
        assert_eq!(b.weight, 1);
    }
    #[test] fn filter_kinds() {
        let f = HttpFilter { kind: "RequestHeaderModifier".into(), config: serde_json::json!({"add": [{"name": "X", "value": "Y"}]}) };
        assert_eq!(f.kind, "RequestHeaderModifier");
    }
}

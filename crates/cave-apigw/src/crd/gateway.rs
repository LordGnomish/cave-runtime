// SPDX-License-Identifier: AGPL-3.0-or-later
//! Gateway + GatewayClass.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayClass {
    pub name: String, pub controller_name: String,
    #[serde(default)] pub description: Option<String>,
}
impl GatewayClass {
    pub fn new(name: &str, controller: &str) -> Self {
        Self { name: name.into(), controller_name: controller.into(), description: None }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayListener {
    pub name: String, pub port: u16, pub protocol: String,
    #[serde(default)] pub hostname: Option<String>,
    #[serde(default)] pub tls: Option<TlsConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TlsConfig {
    pub mode: String, // "Terminate" | "Passthrough"
    #[serde(default)] pub certificate_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Gateway {
    pub name: String, pub gateway_class_name: String,
    pub listeners: Vec<GatewayListener>,
    #[serde(default)] pub addresses: Vec<String>,
}
impl Gateway {
    pub fn new(name: &str, class: &str) -> Self {
        Self { name: name.into(), gateway_class_name: class.into(), listeners: vec![], addresses: vec![] }
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.listeners.is_empty() { return Err("Gateway must have at least one listener".into()); }
        for l in &self.listeners {
            if l.port == 0 { return Err(format!("listener {} has invalid port 0", l.name)); }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn class_construct() {
        let c = GatewayClass::new("cave-class", "cave.io/apigw");
        assert_eq!(c.controller_name, "cave.io/apigw");
    }
    #[test] fn gateway_requires_listener() {
        assert!(Gateway::new("g", "cave-class").validate().is_err());
    }
    #[test] fn listener_port_zero_rejected() {
        let mut g = Gateway::new("g", "cave-class");
        g.listeners.push(GatewayListener { name: "x".into(), port: 0, protocol: "HTTP".into(), hostname: None, tls: None });
        assert!(g.validate().is_err());
    }
    #[test] fn valid_gateway() {
        let mut g = Gateway::new("g", "cave-class");
        g.listeners.push(GatewayListener { name: "x".into(), port: 80, protocol: "HTTP".into(), hostname: None, tls: None });
        g.validate().unwrap();
    }
    #[test] fn tls_listener() {
        let mut g = Gateway::new("g", "cave-class");
        g.listeners.push(GatewayListener {
            name: "tls".into(), port: 443, protocol: "HTTPS".into(),
            hostname: Some("api.example".into()),
            tls: Some(TlsConfig { mode: "Terminate".into(), certificate_refs: vec!["secret/foo".into()] }),
        });
        g.validate().unwrap();
        assert_eq!(g.listeners[0].tls.as_ref().unwrap().mode, "Terminate");
    }
}

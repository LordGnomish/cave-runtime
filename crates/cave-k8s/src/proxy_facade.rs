// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! kube-proxy facade.
//!
//! The real iptables / nftables / eBPF datapath synthesis lives in
//! `cave-kube-proxy`.  cave-k8s tracks the configured mode + the
//! mapping `(Service, EndpointSlice) -> backend rules` at the umbrella
//! level so cavectl + observability dashboards can interrogate the
//! state without touching the datapath.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProxyMode {
    Iptables,
    Nftables,
    Ebpf,
}

impl ProxyMode {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "iptables" => Some(ProxyMode::Iptables),
            "nftables" => Some(ProxyMode::Nftables),
            "ebpf" => Some(ProxyMode::Ebpf),
            _ => None,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            ProxyMode::Iptables => "iptables",
            ProxyMode::Nftables => "nftables",
            ProxyMode::Ebpf => "ebpf",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackendEntry {
    pub service: String,
    pub namespace: String,
    pub virtual_ip: String,
    pub virtual_port: u16,
    pub backends: Vec<(String, u16)>,
    pub session_affinity: bool,
}

impl BackendEntry {
    pub fn key(&self) -> String {
        format!("{}/{}:{}", self.namespace, self.service, self.virtual_port)
    }
    pub fn backend_count(&self) -> usize {
        self.backends.len()
    }
}

pub struct ProxyRegistry {
    mode: ProxyMode,
    entries: std::sync::RwLock<std::collections::BTreeMap<String, BackendEntry>>,
}

impl ProxyRegistry {
    pub fn new(mode: ProxyMode) -> Self {
        Self {
            mode,
            entries: std::sync::RwLock::new(Default::default()),
        }
    }
    pub fn mode(&self) -> ProxyMode {
        self.mode
    }
    pub fn upsert(&self, e: BackendEntry) {
        self.entries.write().expect("proxy lock").insert(e.key(), e);
    }
    pub fn remove(&self, namespace: &str, service: &str, port: u16) -> bool {
        let key = format!("{}/{}:{}", namespace, service, port);
        self.entries.write().expect("proxy lock").remove(&key).is_some()
    }
    pub fn count(&self) -> usize {
        self.entries.read().expect("proxy lock").len()
    }
    pub fn get(&self, namespace: &str, service: &str, port: u16) -> Option<BackendEntry> {
        let key = format!("{}/{}:{}", namespace, service, port);
        self.entries.read().expect("proxy lock").get(&key).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_mode_strings() {
        assert_eq!(ProxyMode::parse("iptables"), Some(ProxyMode::Iptables));
        assert_eq!(ProxyMode::parse("NFTABLES"), Some(ProxyMode::Nftables));
        assert_eq!(ProxyMode::parse("ebpf"), Some(ProxyMode::Ebpf));
        assert!(ProxyMode::parse("ipvs").is_none());
    }

    #[test]
    fn registry_upsert_replaces() {
        let r = ProxyRegistry::new(ProxyMode::Nftables);
        let e = BackendEntry {
            service: "svc".into(),
            namespace: "default".into(),
            virtual_ip: "10.0.0.1".into(),
            virtual_port: 80,
            backends: vec![("10.244.0.1".into(), 8080)],
            session_affinity: false,
        };
        r.upsert(e.clone());
        assert_eq!(r.count(), 1);
        r.upsert(BackendEntry {
            backends: vec![("10.244.0.1".into(), 8080), ("10.244.0.2".into(), 8080)],
            ..e
        });
        let back = r.get("default", "svc", 80).unwrap();
        assert_eq!(back.backend_count(), 2);
    }

    #[test]
    fn remove_returns_false_when_absent() {
        let r = ProxyRegistry::new(ProxyMode::Ebpf);
        assert!(!r.remove("default", "nope", 80));
    }

    #[test]
    fn key_format_stable() {
        let e = BackendEntry {
            service: "x".into(),
            namespace: "n".into(),
            virtual_ip: "1.2.3.4".into(),
            virtual_port: 9090,
            backends: vec![],
            session_affinity: false,
        };
        assert_eq!(e.key(), "n/x:9090");
    }

    #[test]
    fn mode_as_str_stable() {
        assert_eq!(ProxyMode::Iptables.as_str(), "iptables");
        assert_eq!(ProxyMode::Nftables.as_str(), "nftables");
        assert_eq!(ProxyMode::Ebpf.as_str(), "ebpf");
    }

    #[test]
    fn empty_registry_count_zero() {
        let r = ProxyRegistry::new(ProxyMode::Nftables);
        assert_eq!(r.count(), 0);
        assert!(r.get("x", "y", 80).is_none());
    }
}

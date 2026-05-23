// SPDX-License-Identifier: AGPL-3.0-or-later
//! TLSRoute — L4 SNI-based routing.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TlsRoute {
    pub name: String, pub parent_refs: Vec<String>,
    pub hostnames: Vec<String>,
    pub backend_refs: Vec<crate::crd::httproute::HttpBackend>,
}
impl TlsRoute {
    pub fn new(name: &str) -> Self { Self { name: name.into(), parent_refs: vec![], hostnames: vec![], backend_refs: vec![] } }
    pub fn validate(&self) -> Result<(), String> {
        if self.hostnames.is_empty() { return Err("TLSRoute must have at least one hostname (SNI)".into()); }
        if self.backend_refs.is_empty() { return Err("TLSRoute must have backend_refs".into()); }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crd::httproute::HttpBackend;
    #[test] fn no_host_rejected() { assert!(TlsRoute::new("r").validate().is_err()); }
    #[test] fn no_backends_rejected() {
        let mut r = TlsRoute::new("r"); r.hostnames = vec!["api.example".into()];
        assert!(r.validate().is_err());
    }
    #[test] fn valid() {
        let mut r = TlsRoute::new("r");
        r.hostnames = vec!["api.example".into()];
        r.backend_refs = vec![HttpBackend { name: "svc".into(), port: 443, weight: 1 }];
        r.validate().unwrap();
    }
}

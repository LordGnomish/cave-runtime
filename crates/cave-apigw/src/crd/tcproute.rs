// SPDX-License-Identifier: AGPL-3.0-or-later
//! TCPRoute — L4 TCP routing.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TcpRoute {
    pub name: String, pub parent_refs: Vec<String>,
    pub backend_refs: Vec<crate::crd::httproute::HttpBackend>,
}
impl TcpRoute {
    pub fn new(name: &str) -> Self { Self { name: name.into(), parent_refs: vec![], backend_refs: vec![] } }
    pub fn validate(&self) -> Result<(), String> {
        if self.backend_refs.is_empty() { return Err("TCPRoute must have backend_refs".into()); }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crd::httproute::HttpBackend;
    #[test] fn no_backends_rejected() { assert!(TcpRoute::new("r").validate().is_err()); }
    #[test] fn valid() {
        let mut r = TcpRoute::new("r");
        r.backend_refs = vec![HttpBackend { name: "svc".into(), port: 3306, weight: 1 }];
        r.validate().unwrap();
    }
    #[test] fn multiple_backends() {
        let mut r = TcpRoute::new("r");
        r.backend_refs = vec![
            HttpBackend { name: "a".into(), port: 3306, weight: 1 },
            HttpBackend { name: "b".into(), port: 3306, weight: 1 },
        ];
        r.validate().unwrap();
        assert_eq!(r.backend_refs.len(), 2);
    }
}

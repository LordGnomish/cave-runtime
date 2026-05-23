// SPDX-License-Identifier: AGPL-3.0-or-later
//! UDPRoute — L4 UDP routing (DNS, syslog).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UdpRoute {
    pub name: String, pub parent_refs: Vec<String>,
    pub backend_refs: Vec<crate::crd::httproute::HttpBackend>,
}
impl UdpRoute {
    pub fn new(name: &str) -> Self { Self { name: name.into(), parent_refs: vec![], backend_refs: vec![] } }
    pub fn validate(&self) -> Result<(), String> {
        if self.backend_refs.is_empty() { return Err("UDPRoute must have backend_refs".into()); }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crd::httproute::HttpBackend;
    #[test] fn no_backends_rejected() { assert!(UdpRoute::new("r").validate().is_err()); }
    #[test] fn valid() {
        let mut r = UdpRoute::new("r");
        r.backend_refs = vec![HttpBackend { name: "dns".into(), port: 53, weight: 1 }];
        r.validate().unwrap();
    }
    #[test] fn json_round_trip() {
        let mut r = UdpRoute::new("r");
        r.backend_refs = vec![HttpBackend { name: "dns".into(), port: 53, weight: 1 }];
        let s = serde_json::to_string(&r).unwrap();
        let r2: UdpRoute = serde_json::from_str(&s).unwrap();
        assert_eq!(r2.backend_refs[0].name, "dns");
    }
}

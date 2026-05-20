// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Node discovery loop.
//!
//! Mirrors `pkg/nodediscovery/nodediscovery.go`. The agent keeps a
//! local `Node` snapshot describing this host, periodically validates
//! it against the K8s CiliumNode CRD, and pushes changes to the
//! kvstore so other agents can see them.
//!
//! We port the announce/withdraw cycle as a state machine. Wall-clock
//! intervals match upstream defaults.

use crate::cilium::node_mgr::{Node, NodeSource};
use crate::cilium::types::{Cite, TenantId};
use std::collections::BTreeMap;
use std::time::Duration;

/// Default validation interval. Mirrors `validateInterval` upstream.
pub const VALIDATE_INTERVAL: Duration = Duration::from_secs(60);
/// Default kvstore re-announce interval.
pub const ANNOUNCE_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscoveryState {
    Bootstrapping,
    Announced,
    Validated,
    Withdrawn,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DiscoveryError {
    #[error("local node not initialised")]
    NotInitialised,
    #[error("tenant {tenant} cannot mutate node discovery")]
    TenantDenied { tenant: TenantId },
}

/// Per-host node-discovery state machine.
#[derive(Debug)]
pub struct NodeDiscovery {
    pub tenant: TenantId,
    pub local: Option<Node>,
    pub state: DiscoveryState,
    pub announces: u64,
    pub validations: u64,
}

impl NodeDiscovery {
    pub fn new(tenant: TenantId) -> Self {
        Self {
            tenant,
            local: None,
            state: DiscoveryState::Bootstrapping,
            announces: 0,
            validations: 0,
        }
    }

    pub fn set_local(&mut self, name: &str, ipv4: Option<String>, ipv6: Option<String>) {
        self.local = Some(Node {
            name: name.into(),
            cluster: "default".into(),
            source: NodeSource::Local,
            ipv4,
            ipv6,
            labels: BTreeMap::new(),
            identity: 1, // host
        });
    }

    pub fn announce(&mut self) -> Result<&Node, DiscoveryError> {
        let node = self.local.as_ref().ok_or(DiscoveryError::NotInitialised)?;
        self.announces += 1;
        self.state = DiscoveryState::Announced;
        Ok(node)
    }

    pub fn validate(&mut self) -> Result<DiscoveryState, DiscoveryError> {
        if self.local.is_none() {
            return Err(DiscoveryError::NotInitialised);
        }
        self.validations += 1;
        self.state = DiscoveryState::Validated;
        Ok(self.state)
    }

    pub fn withdraw(&mut self) {
        self.state = DiscoveryState::Withdrawn;
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/nodediscovery/nodediscovery.go", "NodeDiscovery");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;

    #[test]
    fn intervals_default_to_one_minute() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/nodediscovery/nodediscovery.go",
            "Intervals",
            "tenant-nd-iv"
        );
        assert_eq!(VALIDATE_INTERVAL, Duration::from_secs(60));
        assert_eq!(ANNOUNCE_INTERVAL, Duration::from_secs(60));
    }

    #[test]
    fn fresh_state_is_bootstrapping() {
        let (_c, t) = cilium_test_ctx!(
            "pkg/nodediscovery/nodediscovery.go",
            "State.Init",
            "tenant-nd-i"
        );
        let nd = NodeDiscovery::new(t);
        assert_eq!(nd.state, DiscoveryState::Bootstrapping);
        assert!(nd.local.is_none());
    }

    #[test]
    fn announce_without_local_node_errors() {
        let (_c, t) = cilium_test_ctx!(
            "pkg/nodediscovery/nodediscovery.go",
            "Announce.NoLocal",
            "tenant-nd-anl"
        );
        let mut nd = NodeDiscovery::new(t);
        let e = nd.announce().unwrap_err();
        assert_eq!(e, DiscoveryError::NotInitialised);
    }

    #[test]
    fn announce_after_set_local_advances_state() {
        let (_c, t) = cilium_test_ctx!(
            "pkg/nodediscovery/nodediscovery.go",
            "Announce.Ok",
            "tenant-nd-ao"
        );
        let mut nd = NodeDiscovery::new(t);
        nd.set_local("host-a", Some("10.0.0.1".into()), None);
        let n = nd.announce().unwrap();
        assert_eq!(n.name, "host-a");
        assert_eq!(nd.state, DiscoveryState::Announced);
        assert_eq!(nd.announces, 1);
    }

    #[test]
    fn validate_increments_counter() {
        let (_c, t) = cilium_test_ctx!(
            "pkg/nodediscovery/nodediscovery.go",
            "Validate",
            "tenant-nd-v"
        );
        let mut nd = NodeDiscovery::new(t);
        nd.set_local("a", Some("1.1.1.1".into()), None);
        nd.validate().unwrap();
        nd.validate().unwrap();
        assert_eq!(nd.validations, 2);
        assert_eq!(nd.state, DiscoveryState::Validated);
    }

    #[test]
    fn validate_without_local_errors() {
        let (_c, t) = cilium_test_ctx!(
            "pkg/nodediscovery/nodediscovery.go",
            "Validate.NoLocal",
            "tenant-nd-vnl"
        );
        let mut nd = NodeDiscovery::new(t);
        let e = nd.validate().unwrap_err();
        assert_eq!(e, DiscoveryError::NotInitialised);
    }

    #[test]
    fn withdraw_moves_to_withdrawn_state() {
        let (_c, t) = cilium_test_ctx!(
            "pkg/nodediscovery/nodediscovery.go",
            "Withdraw",
            "tenant-nd-w"
        );
        let mut nd = NodeDiscovery::new(t);
        nd.set_local("a", Some("1.1.1.1".into()), None);
        nd.announce().unwrap();
        nd.withdraw();
        assert_eq!(nd.state, DiscoveryState::Withdrawn);
    }

    #[test]
    fn local_node_source_is_local() {
        let (_c, t) = cilium_test_ctx!(
            "pkg/nodediscovery/nodediscovery.go",
            "Local.Source",
            "tenant-nd-ls"
        );
        let mut nd = NodeDiscovery::new(t);
        nd.set_local("a", Some("1.1.1.1".into()), None);
        assert_eq!(nd.local.as_ref().unwrap().source, NodeSource::Local);
    }

    #[test]
    fn local_node_identity_is_host_identity() {
        let (_c, t) = cilium_test_ctx!(
            "pkg/nodediscovery/nodediscovery.go",
            "Local.Identity",
            "tenant-nd-li"
        );
        let mut nd = NodeDiscovery::new(t);
        nd.set_local("a", Some("1.1.1.1".into()), None);
        assert_eq!(nd.local.as_ref().unwrap().identity, 1);
    }
}

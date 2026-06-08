// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! CiliumNetworkPolicy CRD + policy controller.
//!
//! Ports the control-plane of cilium's policy engine:
//!   * The `cilium.io/v2` CRD shapes — `CiliumNetworkPolicy`, `Rule`,
//!     `EndpointSelector`, `IngressRule`/`EgressRule`, `PortRule`, the L7
//!     `HTTP` rule (`pkg/policy/api`).
//!   * Numeric **security identities** allocated from label-sets, with the
//!     reserved-identity ordinals from `pkg/identity/numericidentity.go`.
//!   * The **reconciler** (`pkg/policy/repository.go` + `resolve.go`): given
//!     a selected endpoint and the identity universe, it lowers matching
//!     rules into a policy-map (`pkg/maps/policymap`) — the set of
//!     `(peer, port, protocol, L7)` allow entries — applying default-deny
//!     semantics per direction.
//!
//! This is the userspace bookkeeping cilium's agent does before it writes
//! the BPF `cilium_policy` map; the datapath enforcement itself lives in
//! `cave-net::ebpf_sim`.

use std::collections::BTreeMap;

use ipnet::Ipv4Net;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Labels + selectors
// ---------------------------------------------------------------------------

/// An ordered label set (sorted for stable identity hashing).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Labels(pub BTreeMap<String, String>);

impl Labels {
    pub fn new(pairs: &[(&str, &str)]) -> Self {
        Labels(
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        )
    }

    /// Canonical `k=v;k=v;` form — the key under which an identity is
    /// allocated (cilium hashes the sorted label list the same way).
    pub fn canonical(&self) -> String {
        let mut s = String::new();
        for (k, v) in &self.0 {
            s.push_str(k);
            s.push('=');
            s.push_str(v);
            s.push(';');
        }
        s
    }
}

/// `EndpointSelector` — a `matchLabels` subset match.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndpointSelector {
    pub match_labels: BTreeMap<String, String>,
}

impl EndpointSelector {
    pub fn new(pairs: &[(&str, &str)]) -> Self {
        EndpointSelector {
            match_labels: pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    /// Subset match: every `matchLabels` entry must be present and equal.
    /// An empty selector (`{}`) matches every endpoint.
    pub fn matches(&self, labels: &Labels) -> bool {
        self.match_labels
            .iter()
            .all(|(k, v)| labels.0.get(k).map(|lv| lv == v).unwrap_or(false))
    }
}

// ---------------------------------------------------------------------------
// Identities
// ---------------------------------------------------------------------------

/// Reserved numeric identities — ordinals from `numericidentity.go`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum ReservedIdentity {
    Host = 1,
    World = 2,
    Unmanaged = 3,
    Health = 4,
    Init = 5,
    RemoteNode = 6,
    KubeApiServer = 7,
    Ingress = 8,
}

/// Allocated (non-reserved) identities start at this value in cilium.
pub const MIN_ALLOCATED_IDENTITY: u32 = 256;

/// Allocates a stable numeric identity per label-set.
#[derive(Debug, Default)]
pub struct IdentityAllocator {
    by_labels: BTreeMap<String, u32>,
    next: u32,
}

impl IdentityAllocator {
    pub fn new() -> Self {
        IdentityAllocator {
            by_labels: BTreeMap::new(),
            next: MIN_ALLOCATED_IDENTITY,
        }
    }

    /// Idempotent: the same label-set always returns the same identity.
    pub fn allocate(&mut self, labels: &Labels) -> u32 {
        let key = labels.canonical();
        if let Some(id) = self.by_labels.get(&key) {
            return *id;
        }
        let id = self.next;
        self.next += 1;
        self.by_labels.insert(key, id);
        id
    }

    pub fn lookup(&self, labels: &Labels) -> Option<u32> {
        self.by_labels.get(&labels.canonical()).copied()
    }
}

/// A known identity in the universe used during reconcile.
#[derive(Debug, Clone)]
pub struct ResolvedIdentity {
    pub id: u32,
    pub labels: Labels,
}

impl ResolvedIdentity {
    pub fn new(id: u32, labels: Labels) -> Self {
        ResolvedIdentity { id, labels }
    }
}

// ---------------------------------------------------------------------------
// Rule shapes (cilium.io/v2)
// ---------------------------------------------------------------------------

/// L4 protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Protocol {
    Any,
    Tcp,
    Udp,
}

/// `well-known` entities (`pkg/policy/api/entity.go`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Entity {
    All,
    World,
    Host,
    Cluster,
    RemoteNode,
    Health,
    Init,
    Unmanaged,
    KubeApiServer,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct L4Port {
    pub port: u16,
    pub protocol: Protocol,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpRule {
    pub method: Option<String>,
    pub path: Option<String>,
    pub headers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct L7Rules {
    pub http: Vec<HttpRule>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortRule {
    pub ports: Vec<L4Port>,
    pub l7: Option<L7Rules>,
}

#[derive(Debug, Clone, Default)]
pub struct IngressRule {
    pub from_endpoints: Vec<EndpointSelector>,
    pub from_cidr: Vec<Ipv4Net>,
    pub from_entities: Vec<Entity>,
    pub to_ports: Vec<PortRule>,
}

#[derive(Debug, Clone, Default)]
pub struct EgressRule {
    pub to_endpoints: Vec<EndpointSelector>,
    pub to_cidr: Vec<Ipv4Net>,
    pub to_entities: Vec<Entity>,
    pub to_ports: Vec<PortRule>,
}

#[derive(Debug, Clone)]
pub struct Rule {
    pub endpoint_selector: EndpointSelector,
    pub ingress: Vec<IngressRule>,
    pub egress: Vec<EgressRule>,
}

#[derive(Debug, Clone)]
pub struct CiliumNetworkPolicy {
    pub name: String,
    pub namespace: String,
    pub specs: Vec<Rule>,
}

// ---------------------------------------------------------------------------
// Reconcile output
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Ingress,
    Egress,
}

/// A reconciled peer: a security identity, a CIDR, or a reserved entity.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Peer {
    Identity(u32),
    Cidr(Ipv4Net),
    Entity(Entity),
}

/// One row of the lowered policy map.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyMapEntry {
    pub peer: Peer,
    /// `None` means "all ports".
    pub port: Option<u16>,
    pub protocol: Protocol,
    pub l7: Option<L7Rules>,
}

/// The per-direction reconcile result.
#[derive(Debug, Clone)]
pub struct Resolution {
    pub default_deny: bool,
    pub entries: Vec<PolicyMapEntry>,
}

/// Normalised view of one ingress/egress sub-rule used by the resolver.
struct NormRule<'a> {
    selectors: &'a [EndpointSelector],
    cidrs: &'a [Ipv4Net],
    entities: &'a [Entity],
    to_ports: &'a [PortRule],
}

/// Holds the active CNP rules and reconciles them into policy-map entries.
#[derive(Debug, Default)]
pub struct PolicyRepository {
    rules: Vec<Rule>,
}

impl PolicyRepository {
    pub fn add_rule(&mut self, rule: Rule) {
        self.rules.push(rule);
    }

    pub fn add_policy(&mut self, cnp: &CiliumNetworkPolicy) {
        for r in &cnp.specs {
            self.rules.push(r.clone());
        }
    }

    pub fn rule_count(&self) -> usize {
        self.rules.len()
    }

    /// Reconcile all rules selecting `endpoint` for one direction into the
    /// policy map, applying cilium's default-deny semantics.
    pub fn resolve(
        &self,
        endpoint: &Labels,
        dir: Direction,
        peers: &[ResolvedIdentity],
    ) -> Resolution {
        let mut entries: Vec<PolicyMapEntry> = Vec::new();
        let mut default_deny = false;

        for rule in self.rules.iter().filter(|r| r.endpoint_selector.matches(endpoint)) {
            // Normalise the direction-specific sub-rules.
            let norms: Vec<NormRule> = match dir {
                Direction::Ingress => rule
                    .ingress
                    .iter()
                    .map(|i| NormRule {
                        selectors: &i.from_endpoints,
                        cidrs: &i.from_cidr,
                        entities: &i.from_entities,
                        to_ports: &i.to_ports,
                    })
                    .collect(),
                Direction::Egress => rule
                    .egress
                    .iter()
                    .map(|e| NormRule {
                        selectors: &e.to_endpoints,
                        cidrs: &e.to_cidr,
                        entities: &e.to_entities,
                        to_ports: &e.to_ports,
                    })
                    .collect(),
            };

            // Being selected by a rule that has this direction's rules
            // switches the direction into default-deny enforcement.
            if !norms.is_empty() {
                default_deny = true;
            }

            for n in norms {
                // L3: resolve the peer set.
                let mut sel_peers: Vec<Peer> = Vec::new();
                for sel in n.selectors {
                    for p in peers {
                        if sel.matches(&p.labels) {
                            sel_peers.push(Peer::Identity(p.id));
                        }
                    }
                }
                for c in n.cidrs {
                    sel_peers.push(Peer::Cidr(*c));
                }
                for e in n.entities {
                    sel_peers.push(Peer::Entity(*e));
                }
                // No L3 selector at all means "any source" on those ports.
                if sel_peers.is_empty() {
                    sel_peers.push(Peer::Entity(Entity::All));
                }

                // L4 × peers.
                if n.to_ports.is_empty() {
                    for peer in &sel_peers {
                        push_unique(
                            &mut entries,
                            PolicyMapEntry {
                                peer: peer.clone(),
                                port: None,
                                protocol: Protocol::Any,
                                l7: None,
                            },
                        );
                    }
                } else {
                    for pr in n.to_ports {
                        for port in &pr.ports {
                            for peer in &sel_peers {
                                push_unique(
                                    &mut entries,
                                    PolicyMapEntry {
                                        peer: peer.clone(),
                                        port: Some(port.port),
                                        protocol: port.protocol,
                                        l7: pr.l7.clone(),
                                    },
                                );
                            }
                        }
                    }
                }
            }
        }

        // Deterministic ordering for callers/tests.
        entries.sort_by(|a, b| {
            a.peer
                .cmp(&b.peer)
                .then(a.port.cmp(&b.port))
                .then(a.protocol.cmp(&b.protocol))
        });

        Resolution {
            default_deny,
            entries,
        }
    }
}

fn push_unique(entries: &mut Vec<PolicyMapEntry>, e: PolicyMapEntry) {
    if !entries.contains(&e) {
        entries.push(e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_label_form_is_sorted() {
        let l = Labels::new(&[("b", "2"), ("a", "1")]);
        assert_eq!(l.canonical(), "a=1;b=2;");
    }

    #[test]
    fn toports_only_rule_allows_any_source() {
        let mut repo = PolicyRepository::default();
        repo.add_rule(Rule {
            endpoint_selector: EndpointSelector::new(&[("app", "web")]),
            ingress: vec![IngressRule {
                from_endpoints: vec![],
                from_cidr: vec![],
                from_entities: vec![],
                to_ports: vec![PortRule {
                    ports: vec![L4Port {
                        port: 80,
                        protocol: Protocol::Tcp,
                    }],
                    l7: None,
                }],
            }],
            egress: vec![],
        });
        let r = repo.resolve(&Labels::new(&[("app", "web")]), Direction::Ingress, &[]);
        assert!(r.default_deny);
        assert_eq!(r.entries.len(), 1);
        assert_eq!(r.entries[0].peer, Peer::Entity(Entity::All));
        assert_eq!(r.entries[0].port, Some(80));
    }
}

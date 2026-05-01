//! L3/L4 NetworkPolicy + CiliumNetworkPolicy evaluator.
//!
//! Mirrors `pkg/policy/api/rule.go` (Rule, Ingress, Egress), the entity
//! table from `pkg/policy/api/entity.go`, the selector model from
//! `pkg/policy/api/selector.go`, the CIDR rule model from
//! `pkg/policy/api/cidr.go`, the ICMP rule model from
//! `pkg/policy/api/icmp.go`, and the per-endpoint distillation from
//! `pkg/policy/distillery.go`.
//!
//! Semantics (faithful to upstream):
//!
//! * A [`Rule`] applies to an endpoint when its [`EndpointSelector`] matches
//!   the endpoint's label set.
//! * If *any* matching rule has a non-empty `ingress` list, ingress is
//!   *enforced* for that endpoint (default-deny). Same for egress.
//! * `from_entities = [Entity::All]` is the only sentinel that allows from
//!   every identity including world. `from_entities = [Entity::Cluster]`
//!   allows from the cluster's own identities (everything except world).
//! * `to_ports.ports = []` means *any port*; an empty `to_ports` block
//!   itself contributes only an L3 allow (any port).
//! * `cidr_rule.except` is *subtracted* from `cidr` — an IP that is in cidr
//!   AND in any except block is **not** matched.
//! * The distilled [`PolicyMap`] keys (peer_identity, port, proto, dir) and
//!   maps them to an [`Allow`]/[`Deny`] verdict plus an optional L7
//!   redirect port (mirrors upstream `MapStateEntry`).

use crate::cilium::identity::{LabelSet, ID_HEALTH, ID_HOST, ID_INIT, ID_REMOTE_NODE, ID_UNMANAGED, ID_WORLD};
use crate::cilium::types::{Cite, TenantId};
use ipnet::IpNet;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::IpAddr;
use std::str::FromStr;

// ───────────────────────── Reserved entities ─────────────────────────────────
//
// Cilium reserves identity numbers 1..=255. The numbers below mirror
// `pkg/identity/numericidentity.go` constants. `Entity::All` is the
// `reserved:all` sentinel that selects every identity (including world).

pub const ID_ALL: u32 = 0;
pub const ID_KUBE_APISERVER: u32 = 7;
pub const ID_INGRESS: u32 = 8;
pub const ID_WORLD_IPV4: u32 = 9;
pub const ID_WORLD_IPV6: u32 = 10;
pub const ID_ENCRYPTED_OVERLAY: u32 = 11;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Entity {
    All,
    World,
    WorldIPv4,
    WorldIPv6,
    Host,
    RemoteNode,
    Cluster,
    Init,
    Unmanaged,
    Health,
    KubeApiServer,
    Ingress,
}

impl Entity {
    /// Map an entity to its reserved numeric identity. `Entity::Cluster` has
    /// no single numeric identity — it represents *the set of in-cluster
    /// identities*, distinct from world. Returns `None` for that case.
    pub fn to_identity(self) -> Option<u32> {
        Some(match self {
            Entity::All => ID_ALL,
            Entity::World => ID_WORLD,
            Entity::WorldIPv4 => ID_WORLD_IPV4,
            Entity::WorldIPv6 => ID_WORLD_IPV6,
            Entity::Host => ID_HOST,
            Entity::RemoteNode => ID_REMOTE_NODE,
            Entity::Init => ID_INIT,
            Entity::Unmanaged => ID_UNMANAGED,
            Entity::Health => ID_HEALTH,
            Entity::KubeApiServer => ID_KUBE_APISERVER,
            Entity::Ingress => ID_INGRESS,
            Entity::Cluster => return None,
        })
    }
}

// ───────────────────────── Endpoint selector ─────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SelectorOp {
    In,
    NotIn,
    Exists,
    DoesNotExist,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatchExpression {
    pub key: String,
    pub op: SelectorOp,
    pub values: Vec<String>,
}

/// Mirrors `EndpointSelector` in `pkg/policy/api/selector.go`. Combines
/// `match_labels` (AND semantics, exact equality) with `match_expressions`
/// (also AND). An empty selector matches all endpoints.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndpointSelector {
    pub match_labels: HashMap<String, String>,
    pub match_expressions: Vec<MatchExpression>,
}

impl EndpointSelector {
    pub fn empty() -> Self {
        Self::default()
    }

    /// True if this selector matches the given label set.
    pub fn matches(&self, labels: &LabelSet) -> bool {
        let map: HashMap<&str, &str> = labels.pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        for (k, v) in &self.match_labels {
            match map.get(k.as_str()) {
                Some(have) if *have == v.as_str() => {}
                _ => return false,
            }
        }
        for me in &self.match_expressions {
            let have = map.get(me.key.as_str()).copied();
            let ok = match me.op {
                SelectorOp::In => match have {
                    Some(v) => me.values.iter().any(|x| x == v),
                    None => false,
                },
                SelectorOp::NotIn => match have {
                    Some(v) => !me.values.iter().any(|x| x == v),
                    None => true,
                },
                SelectorOp::Exists => have.is_some(),
                SelectorOp::DoesNotExist => have.is_none(),
            };
            if !ok {
                return false;
            }
        }
        true
    }
}

// ───────────────────────── CIDR rule ─────────────────────────────────────────

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PolicyError {
    #[error("invalid CIDR `{0}`")]
    BadCidr(String),
    #[error("tenant {tenant} cannot evaluate policy owned by another tenant")]
    TenantDenied { tenant: TenantId },
    #[error("identity {0} could not be resolved to a label set")]
    UnknownIdentity(u32),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CidrRule {
    pub cidr: String,
    pub except: Vec<String>,
}

impl CidrRule {
    pub fn new(cidr: impl Into<String>) -> Self {
        Self { cidr: cidr.into(), except: Vec::new() }
    }
    pub fn with_except<I, S>(mut self, except: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.except = except.into_iter().map(Into::into).collect();
        self
    }
    /// True if `ip` falls inside `cidr` and *not* inside any `except` block.
    pub fn contains(&self, ip: IpAddr) -> Result<bool, PolicyError> {
        let net = IpNet::from_str(&self.cidr).map_err(|_| PolicyError::BadCidr(self.cidr.clone()))?;
        if !net.contains(&ip) {
            return Ok(false);
        }
        for ex in &self.except {
            let exnet = IpNet::from_str(ex).map_err(|_| PolicyError::BadCidr(ex.clone()))?;
            if exnet.contains(&ip) {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

// ───────────────────────── L4 / ICMP ─────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum L4Protocol {
    TCP,
    UDP,
    SCTP,
    ICMP,
    /// `Any` matches TCP, UDP and SCTP simultaneously (mirrors upstream
    /// `ProtoAny`). Cilium does **not** include ICMP in `Any`.
    Any,
}

impl L4Protocol {
    /// True if this protocol value (from a rule) covers `wire` (the actual
    /// packet protocol). Mirrors `protoMatch` in upstream.
    pub fn covers(self, wire: L4Protocol) -> bool {
        match (self, wire) {
            (a, b) if a == b => true,
            (L4Protocol::Any, L4Protocol::TCP)
            | (L4Protocol::Any, L4Protocol::UDP)
            | (L4Protocol::Any, L4Protocol::SCTP) => true,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PortProtocol {
    /// 0 means *any port*. Mirrors upstream `Port: ""` interpretation.
    pub port: u16,
    pub protocol: L4Protocol,
}

impl PortProtocol {
    pub fn new(port: u16, protocol: L4Protocol) -> Self {
        Self { port, protocol }
    }
    /// True if this rule port covers `(wire_port, wire_proto)`.
    pub fn covers(self, wire_port: u16, wire_proto: L4Protocol) -> bool {
        if !self.protocol.covers(wire_proto) {
            return false;
        }
        self.port == 0 || self.port == wire_port
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum IcmpFamily {
    V4,
    V6,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct IcmpRule {
    pub family: IcmpFamily,
    pub icmp_type: u8,
}

impl IcmpRule {
    pub fn matches(&self, family: IcmpFamily, icmp_type: u8) -> bool {
        self.family == family && self.icmp_type == icmp_type
    }
}

// ───────────────────────── Rule structures ───────────────────────────────────

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortRule {
    pub ports: Vec<PortProtocol>,
    /// Optional L7 proxy redirect port. Mirrors `MapStateEntry.proxyPort`.
    pub l7_redirect_port: Option<u16>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IngressRule {
    pub from_endpoints: Vec<EndpointSelector>,
    pub from_cidr: Vec<CidrRule>,
    pub from_entities: Vec<Entity>,
    /// `from_requires` is an additional AND filter on the *peer* — only
    /// peers whose labels also match these selectors are allowed.
    pub from_requires: Vec<EndpointSelector>,
    pub to_ports: Vec<PortRule>,
    pub icmps: Vec<IcmpRule>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EgressRule {
    pub to_endpoints: Vec<EndpointSelector>,
    pub to_cidr: Vec<CidrRule>,
    pub to_entities: Vec<Entity>,
    pub to_requires: Vec<EndpointSelector>,
    pub to_ports: Vec<PortRule>,
    pub icmps: Vec<IcmpRule>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    pub name: String,
    pub tenant: TenantId,
    pub endpoint_selector: EndpointSelector,
    pub ingress: Vec<IngressRule>,
    pub egress: Vec<EgressRule>,
    /// Free-form labels the rule itself carries (e.g. `derived-from=cnp`).
    pub labels: HashMap<String, String>,
}

impl Rule {
    pub fn new(name: impl Into<String>, tenant: TenantId, sel: EndpointSelector) -> Self {
        Self {
            name: name.into(),
            tenant,
            endpoint_selector: sel,
            ingress: Vec::new(),
            egress: Vec::new(),
            labels: HashMap::new(),
        }
    }
    pub fn applies_to(&self, labels: &LabelSet) -> bool {
        self.endpoint_selector.matches(labels)
    }
}

// ───────────────────────── Enforcement modes ─────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PolicyEnforcementMode {
    /// Default mode: enforce only when at least one rule matches the endpoint.
    Default,
    /// Always enforce (default-deny everything not explicitly allowed).
    Always,
    /// Never enforce — every packet is allowed.
    Never,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Direction {
    Ingress,
    Egress,
}

// ───────────────────────── PolicyMap ─────────────────────────────────────────

/// Mirrors `policymap.PolicyKey` in upstream — the eBPF map key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PolicyKey {
    pub peer_identity: u32,
    pub port: u16,
    pub protocol: L4Protocol,
    pub direction: Direction,
}

/// Mirrors `policymap.PolicyEntry` (`MapStateEntry`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyEntry {
    pub verdict: Verdict,
    pub l7_redirect_port: Option<u16>,
    pub audit: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Verdict {
    Allow,
    Deny,
}

#[derive(Debug, Clone, Default)]
pub struct PolicyMap {
    /// True if ingress is *enforced* for this endpoint (i.e. default-deny).
    pub ingress_enforced: bool,
    pub egress_enforced: bool,
    pub entries: HashMap<PolicyKey, PolicyEntry>,
}

impl PolicyMap {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn allow(&mut self, key: PolicyKey, l7: Option<u16>) {
        self.entries.insert(
            key,
            PolicyEntry { verdict: Verdict::Allow, l7_redirect_port: l7, audit: false },
        );
    }
    pub fn deny(&mut self, key: PolicyKey) {
        self.entries.insert(key, PolicyEntry { verdict: Verdict::Deny, l7_redirect_port: None, audit: false });
    }
    /// Look up the verdict for a wire packet. Mirrors the kernel-side
    /// `policy_can_access` walk: explicit deny > explicit allow with port
    /// > explicit allow on (peer, any-port) > default-deny if enforced.
    pub fn lookup(&self, peer: u32, port: u16, proto: L4Protocol, dir: Direction) -> PolicyEntry {
        // 1. Exact match on (peer, port, proto, dir).
        if let Some(&entry) = self.entries.get(&PolicyKey {
            peer_identity: peer, port, protocol: proto, direction: dir,
        }) {
            return entry;
        }
        // 2. Allow on (peer, port=0, exact-proto).
        if let Some(&entry) = self.entries.get(&PolicyKey {
            peer_identity: peer, port: 0, protocol: proto, direction: dir,
        }) {
            return entry;
        }
        // 3. Allow on (peer, port, Any-proto).
        if let Some(&entry) = self.entries.get(&PolicyKey {
            peer_identity: peer, port, protocol: L4Protocol::Any, direction: dir,
        }) {
            return entry;
        }
        // 4. Allow on (peer, any-port, Any-proto).
        if let Some(&entry) = self.entries.get(&PolicyKey {
            peer_identity: peer, port: 0, protocol: L4Protocol::Any, direction: dir,
        }) {
            return entry;
        }
        // 5. World fallback for non-cluster peers.
        if let Some(&entry) = self.entries.get(&PolicyKey {
            peer_identity: ID_ALL, port: 0, protocol: L4Protocol::Any, direction: dir,
        }) {
            return entry;
        }
        // 6. Default: enforced direction → Deny; otherwise → Allow.
        let enforced = match dir {
            Direction::Ingress => self.ingress_enforced,
            Direction::Egress => self.egress_enforced,
        };
        PolicyEntry {
            verdict: if enforced { Verdict::Deny } else { Verdict::Allow },
            l7_redirect_port: None,
            audit: false,
        }
    }
}

// ───────────────────────── Repository + Distillery ───────────────────────────

#[derive(Debug, Clone, Default)]
pub struct PolicyRepository {
    pub rules: Vec<Rule>,
}

impl PolicyRepository {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn add(&mut self, rule: Rule) {
        self.rules.push(rule);
    }
    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.rules.len();
        self.rules.retain(|r| r.name != name);
        before != self.rules.len()
    }
    pub fn len(&self) -> usize {
        self.rules.len()
    }
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }
}

/// Resolves a peer's identity → label set (used by `from_requires` /
/// `to_requires` plus selector-based peer matching). Mirrors the
/// `IdentityCache` lookup in `pkg/policy/distillery.go`.
pub trait IdentityResolver {
    fn labels_for(&self, identity: u32) -> Option<LabelSet>;
    /// All known identities (cluster-local). Used to expand
    /// `Entity::Cluster` and selectors on the peer side.
    fn all_identities(&self) -> Vec<u32>;
}

/// In-memory implementation suitable for tests and the cave userspace
/// dataplane.
#[derive(Debug, Default, Clone)]
pub struct InMemoryIdentityResolver {
    map: HashMap<u32, LabelSet>,
}

impl InMemoryIdentityResolver {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn insert(&mut self, identity: u32, labels: LabelSet) {
        self.map.insert(identity, labels);
    }
}

impl IdentityResolver for InMemoryIdentityResolver {
    fn labels_for(&self, identity: u32) -> Option<LabelSet> {
        self.map.get(&identity).cloned()
    }
    fn all_identities(&self) -> Vec<u32> {
        let mut v: Vec<u32> = self.map.keys().copied().collect();
        v.sort();
        v
    }
}

/// Distill the rules in `repo` into a [`PolicyMap`] for an endpoint with
/// the given label set, identity, and tenant.
///
/// Mirrors `pkg/policy/distillery.go::cachedSelectorPolicy.distillPolicy`.
pub fn distill(
    repo: &PolicyRepository,
    tenant: &TenantId,
    endpoint_labels: &LabelSet,
    mode: PolicyEnforcementMode,
    resolver: &dyn IdentityResolver,
) -> Result<PolicyMap, PolicyError> {
    let mut map = PolicyMap::new();

    if matches!(mode, PolicyEnforcementMode::Always) {
        map.ingress_enforced = true;
        map.egress_enforced = true;
    }
    if matches!(mode, PolicyEnforcementMode::Never) {
        return Ok(map); // empty entries; nothing enforced
    }

    for rule in &repo.rules {
        if &rule.tenant != tenant {
            return Err(PolicyError::TenantDenied { tenant: tenant.clone() });
        }
        if !rule.applies_to(endpoint_labels) {
            continue;
        }

        if !rule.ingress.is_empty() && matches!(mode, PolicyEnforcementMode::Default) {
            map.ingress_enforced = true;
        }
        if !rule.egress.is_empty() && matches!(mode, PolicyEnforcementMode::Default) {
            map.egress_enforced = true;
        }

        for ig in &rule.ingress {
            distill_ingress(ig, &mut map, resolver);
        }
        for eg in &rule.egress {
            distill_egress(eg, &mut map, resolver);
        }
    }

    Ok(map)
}

fn distill_ingress(rule: &IngressRule, map: &mut PolicyMap, resolver: &dyn IdentityResolver) {
    let peers = collect_peers_ingress(rule, resolver);
    write_port_entries(&peers, &rule.to_ports, &rule.icmps, Direction::Ingress, map);
}

fn distill_egress(rule: &EgressRule, map: &mut PolicyMap, resolver: &dyn IdentityResolver) {
    let peers = collect_peers_egress(rule, resolver);
    write_port_entries(&peers, &rule.to_ports, &rule.icmps, Direction::Egress, map);
}

fn collect_peers_ingress(rule: &IngressRule, resolver: &dyn IdentityResolver) -> Vec<u32> {
    let mut peers = Vec::new();
    expand_entities(&rule.from_entities, resolver, &mut peers);
    expand_endpoint_selectors(&rule.from_endpoints, &rule.from_requires, resolver, &mut peers);
    if !rule.from_cidr.is_empty() {
        // CIDR rules contribute world by default — the actual IP filter
        // happens at lookup time. We model this by emitting the world id
        // unless the rule restricts to IPv4/IPv6 explicitly via except.
        peers.push(ID_WORLD);
    }
    peers.sort();
    peers.dedup();
    peers
}

fn collect_peers_egress(rule: &EgressRule, resolver: &dyn IdentityResolver) -> Vec<u32> {
    let mut peers = Vec::new();
    expand_entities(&rule.to_entities, resolver, &mut peers);
    expand_endpoint_selectors(&rule.to_endpoints, &rule.to_requires, resolver, &mut peers);
    if !rule.to_cidr.is_empty() {
        peers.push(ID_WORLD);
    }
    peers.sort();
    peers.dedup();
    peers
}

fn expand_entities(entities: &[Entity], resolver: &dyn IdentityResolver, out: &mut Vec<u32>) {
    for e in entities {
        match e {
            Entity::Cluster => {
                // Cluster entity expands to every known cluster-local id.
                for id in resolver.all_identities() {
                    out.push(id);
                }
            }
            other => {
                if let Some(id) = other.to_identity() {
                    out.push(id);
                }
            }
        }
    }
}

fn expand_endpoint_selectors(
    selectors: &[EndpointSelector],
    requires: &[EndpointSelector],
    resolver: &dyn IdentityResolver,
    out: &mut Vec<u32>,
) {
    if selectors.is_empty() {
        return;
    }
    for id in resolver.all_identities() {
        let labels = match resolver.labels_for(id) {
            Some(l) => l,
            None => continue,
        };
        let primary_match = selectors.iter().any(|s| s.matches(&labels));
        if !primary_match {
            continue;
        }
        // requires are AND filters on the peer.
        let requires_ok = requires.iter().all(|r| r.matches(&labels));
        if !requires_ok {
            continue;
        }
        out.push(id);
    }
}

fn write_port_entries(
    peers: &[u32],
    to_ports: &[PortRule],
    icmps: &[IcmpRule],
    dir: Direction,
    map: &mut PolicyMap,
) {
    if to_ports.is_empty() && icmps.is_empty() {
        // L3-only allow → (peer, port=0, Any).
        for &peer in peers {
            let key = PolicyKey { peer_identity: peer, port: 0, protocol: L4Protocol::Any, direction: dir };
            map.allow(key, None);
        }
        return;
    }
    for pr in to_ports {
        if pr.ports.is_empty() {
            for &peer in peers {
                let key = PolicyKey { peer_identity: peer, port: 0, protocol: L4Protocol::Any, direction: dir };
                map.allow(key, pr.l7_redirect_port);
            }
            continue;
        }
        for pp in &pr.ports {
            for &peer in peers {
                let key = PolicyKey {
                    peer_identity: peer, port: pp.port, protocol: pp.protocol, direction: dir,
                };
                map.allow(key, pr.l7_redirect_port);
            }
        }
    }
    for ic in icmps {
        for &peer in peers {
            // ICMP type encoded as the port number; family carried by proto = ICMP.
            let key = PolicyKey {
                peer_identity: peer,
                port: ic.icmp_type as u16,
                protocol: L4Protocol::ICMP,
                direction: dir,
            };
            map.allow(key, None);
        }
    }
    let _ = peers;
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/policy/api/rule.go", "Rule");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium::identity::{LabelSet, ID_HOST, ID_WORLD, MIN_LOCAL_IDENTITY};
    use crate::cilium_test_ctx;
    use std::net::Ipv4Addr;

    fn ls(pairs: &[(&str, &str)]) -> LabelSet {
        LabelSet::from_iter(pairs.iter().map(|(k, v)| (*k, *v)))
    }

    fn endpoint_sel(pairs: &[(&str, &str)]) -> EndpointSelector {
        EndpointSelector {
            match_labels: pairs.iter().map(|(k, v)| ((*k).into(), (*v).into())).collect(),
            match_expressions: Vec::new(),
        }
    }

    // ── Entity → identity mapping ────────────────────────────────────────────

    #[test]
    fn entity_world_resolves_to_id_world() {
        let (_c, _t) = cilium_test_ctx!("pkg/policy/api/entity.go", "EntityWorld", "tenant-pol-ent-world");
        assert_eq!(Entity::World.to_identity(), Some(ID_WORLD));
    }

    #[test]
    fn entity_host_resolves_to_id_host() {
        let (_c, _t) = cilium_test_ctx!("pkg/policy/api/entity.go", "EntityHost", "tenant-pol-ent-host");
        assert_eq!(Entity::Host.to_identity(), Some(ID_HOST));
    }

    #[test]
    fn entity_kube_apiserver_resolves_to_id_7() {
        let (_c, _t) = cilium_test_ctx!("pkg/policy/api/entity.go", "EntityKubeAPIServer", "tenant-pol-ent-kas");
        assert_eq!(Entity::KubeApiServer.to_identity(), Some(ID_KUBE_APISERVER));
    }

    #[test]
    fn entity_ingress_resolves_to_id_8() {
        let (_c, _t) = cilium_test_ctx!("pkg/policy/api/entity.go", "EntityIngress", "tenant-pol-ent-ing");
        assert_eq!(Entity::Ingress.to_identity(), Some(ID_INGRESS));
    }

    #[test]
    fn entity_remote_node_resolves_to_id_remote_node() {
        let (_c, _t) = cilium_test_ctx!("pkg/policy/api/entity.go", "EntityRemoteNode", "tenant-pol-ent-rn");
        assert_eq!(Entity::RemoteNode.to_identity(), Some(ID_REMOTE_NODE));
    }

    #[test]
    fn entity_health_resolves_to_id_health() {
        let (_c, _t) = cilium_test_ctx!("pkg/policy/api/entity.go", "EntityHealth", "tenant-pol-ent-hlth");
        assert_eq!(Entity::Health.to_identity(), Some(ID_HEALTH));
    }

    #[test]
    fn entity_init_resolves_to_id_init() {
        let (_c, _t) = cilium_test_ctx!("pkg/policy/api/entity.go", "EntityInit", "tenant-pol-ent-init");
        assert_eq!(Entity::Init.to_identity(), Some(ID_INIT));
    }

    #[test]
    fn entity_unmanaged_resolves_to_id_unmanaged() {
        let (_c, _t) = cilium_test_ctx!("pkg/policy/api/entity.go", "EntityUnmanaged", "tenant-pol-ent-unm");
        assert_eq!(Entity::Unmanaged.to_identity(), Some(ID_UNMANAGED));
    }

    #[test]
    fn entity_world_ipv4_resolves_to_id_9() {
        let (_c, _t) = cilium_test_ctx!("pkg/policy/api/entity.go", "EntityWorldIPv4", "tenant-pol-ent-w4");
        assert_eq!(Entity::WorldIPv4.to_identity(), Some(ID_WORLD_IPV4));
    }

    #[test]
    fn entity_world_ipv6_resolves_to_id_10() {
        let (_c, _t) = cilium_test_ctx!("pkg/policy/api/entity.go", "EntityWorldIPv6", "tenant-pol-ent-w6");
        assert_eq!(Entity::WorldIPv6.to_identity(), Some(ID_WORLD_IPV6));
    }

    #[test]
    fn entity_all_resolves_to_id_zero() {
        let (_c, _t) = cilium_test_ctx!("pkg/policy/api/entity.go", "EntityAll", "tenant-pol-ent-all");
        assert_eq!(Entity::All.to_identity(), Some(ID_ALL));
    }

    #[test]
    fn entity_cluster_has_no_single_identity() {
        let (_c, _t) = cilium_test_ctx!("pkg/policy/api/entity.go", "EntityCluster", "tenant-pol-ent-cl");
        assert_eq!(Entity::Cluster.to_identity(), None);
    }

    // ── EndpointSelector ─────────────────────────────────────────────────────

    #[test]
    fn endpoint_selector_match_labels_exact() {
        let (_c, _t) = cilium_test_ctx!("pkg/policy/api/selector.go", "EndpointSelector.Matches", "tenant-pol-sel-exact");
        let sel = endpoint_sel(&[("app", "web")]);
        assert!(sel.matches(&ls(&[("app", "web")])));
    }

    #[test]
    fn endpoint_selector_match_labels_subset_match() {
        let (_c, _t) = cilium_test_ctx!("pkg/policy/api/selector.go", "EndpointSelector.Matches", "tenant-pol-sel-sub");
        let sel = endpoint_sel(&[("app", "web")]);
        assert!(sel.matches(&ls(&[("app", "web"), ("env", "prod")])));
    }

    #[test]
    fn endpoint_selector_match_labels_mismatch() {
        let (_c, _t) = cilium_test_ctx!("pkg/policy/api/selector.go", "EndpointSelector.Matches", "tenant-pol-sel-mm");
        let sel = endpoint_sel(&[("app", "web")]);
        assert!(!sel.matches(&ls(&[("app", "api")])));
    }

    #[test]
    fn endpoint_selector_empty_matches_all() {
        let (_c, _t) = cilium_test_ctx!("pkg/policy/api/selector.go", "EndpointSelector.Matches", "tenant-pol-sel-empty");
        let sel = EndpointSelector::empty();
        assert!(sel.matches(&ls(&[("app", "web")])));
        assert!(sel.matches(&ls(&[])));
    }

    #[test]
    fn endpoint_selector_in_expression() {
        let (_c, _t) = cilium_test_ctx!("pkg/policy/api/selector.go", "MatchExpression(In)", "tenant-pol-sel-in");
        let sel = EndpointSelector {
            match_labels: HashMap::new(),
            match_expressions: vec![MatchExpression {
                key: "env".into(), op: SelectorOp::In, values: vec!["prod".into(), "stage".into()],
            }],
        };
        assert!(sel.matches(&ls(&[("env", "prod")])));
        assert!(sel.matches(&ls(&[("env", "stage")])));
        assert!(!sel.matches(&ls(&[("env", "dev")])));
    }

    #[test]
    fn endpoint_selector_notin_expression() {
        let (_c, _t) = cilium_test_ctx!("pkg/policy/api/selector.go", "MatchExpression(NotIn)", "tenant-pol-sel-notin");
        let sel = EndpointSelector {
            match_labels: HashMap::new(),
            match_expressions: vec![MatchExpression {
                key: "tier".into(), op: SelectorOp::NotIn, values: vec!["test".into()],
            }],
        };
        assert!(sel.matches(&ls(&[("tier", "prod")])));
        assert!(!sel.matches(&ls(&[("tier", "test")])));
        // NotIn semantics: missing key → match (key is not in the disallowed set).
        assert!(sel.matches(&ls(&[("other", "x")])));
    }

    #[test]
    fn endpoint_selector_exists_expression() {
        let (_c, _t) = cilium_test_ctx!("pkg/policy/api/selector.go", "MatchExpression(Exists)", "tenant-pol-sel-ex");
        let sel = EndpointSelector {
            match_labels: HashMap::new(),
            match_expressions: vec![MatchExpression { key: "k".into(), op: SelectorOp::Exists, values: vec![] }],
        };
        assert!(sel.matches(&ls(&[("k", "anything")])));
        assert!(!sel.matches(&ls(&[("other", "v")])));
    }

    #[test]
    fn endpoint_selector_doesnotexist_expression() {
        let (_c, _t) = cilium_test_ctx!("pkg/policy/api/selector.go", "MatchExpression(DoesNotExist)", "tenant-pol-sel-nx");
        let sel = EndpointSelector {
            match_labels: HashMap::new(),
            match_expressions: vec![MatchExpression { key: "k".into(), op: SelectorOp::DoesNotExist, values: vec![] }],
        };
        assert!(sel.matches(&ls(&[])));
        assert!(!sel.matches(&ls(&[("k", "v")])));
    }

    #[test]
    fn endpoint_selector_combines_match_labels_and_expressions() {
        let (_c, _t) = cilium_test_ctx!("pkg/policy/api/selector.go", "EndpointSelector.Matches", "tenant-pol-sel-comb");
        let sel = EndpointSelector {
            match_labels: HashMap::from([("app".into(), "web".into())]),
            match_expressions: vec![MatchExpression {
                key: "env".into(), op: SelectorOp::In, values: vec!["prod".into()],
            }],
        };
        assert!(sel.matches(&ls(&[("app", "web"), ("env", "prod")])));
        assert!(!sel.matches(&ls(&[("app", "web"), ("env", "dev")])));
        assert!(!sel.matches(&ls(&[("app", "api"), ("env", "prod")])));
    }

    // ── CIDR ─────────────────────────────────────────────────────────────────

    #[test]
    fn cidr_rule_contains_ipv4_in_range() {
        let (_c, _t) = cilium_test_ctx!("pkg/policy/api/cidr.go", "CIDRRule", "tenant-pol-cidr-in");
        let r = CidrRule::new("10.0.0.0/8");
        assert!(r.contains(IpAddr::V4(Ipv4Addr::new(10, 1, 2, 3))).unwrap());
    }

    #[test]
    fn cidr_rule_contains_ipv4_out_of_range() {
        let (_c, _t) = cilium_test_ctx!("pkg/policy/api/cidr.go", "CIDRRule", "tenant-pol-cidr-out");
        let r = CidrRule::new("10.0.0.0/8");
        assert!(!r.contains(IpAddr::V4(Ipv4Addr::new(11, 0, 0, 1))).unwrap());
    }

    #[test]
    fn cidr_rule_except_blocks_subnet() {
        let (_c, _t) = cilium_test_ctx!("pkg/policy/api/cidr.go", "CIDRRule.Except", "tenant-pol-cidr-ex");
        let r = CidrRule::new("10.0.0.0/8").with_except(["10.10.0.0/16"]);
        assert!(r.contains(IpAddr::V4(Ipv4Addr::new(10, 1, 0, 1))).unwrap());
        assert!(!r.contains(IpAddr::V4(Ipv4Addr::new(10, 10, 0, 1))).unwrap());
    }

    #[test]
    fn cidr_rule_invalid_returns_error() {
        let (_c, _t) = cilium_test_ctx!("pkg/policy/api/cidr.go", "CIDRRule.Validate", "tenant-pol-cidr-bad");
        let r = CidrRule::new("not-a-cidr");
        let err = r.contains(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))).unwrap_err();
        assert_eq!(err, PolicyError::BadCidr("not-a-cidr".into()));
    }

    #[test]
    fn cidr_rule_ipv6_in_range() {
        let (_c, _t) = cilium_test_ctx!("pkg/policy/api/cidr.go", "CIDRRule.IPv6", "tenant-pol-cidr-v6");
        let r = CidrRule::new("2001:db8::/32");
        let ip: IpAddr = "2001:db8::1".parse().unwrap();
        assert!(r.contains(ip).unwrap());
    }

    // ── L4Protocol / PortProtocol ────────────────────────────────────────────

    #[test]
    fn l4_proto_any_covers_tcp_udp_sctp_but_not_icmp() {
        let (_c, _t) = cilium_test_ctx!("pkg/policy/api/ports.go", "ProtoAny", "tenant-pol-l4-any");
        assert!(L4Protocol::Any.covers(L4Protocol::TCP));
        assert!(L4Protocol::Any.covers(L4Protocol::UDP));
        assert!(L4Protocol::Any.covers(L4Protocol::SCTP));
        assert!(!L4Protocol::Any.covers(L4Protocol::ICMP));
    }

    #[test]
    fn l4_proto_specific_only_covers_self() {
        let (_c, _t) = cilium_test_ctx!("pkg/policy/api/ports.go", "Protocol", "tenant-pol-l4-self");
        assert!(L4Protocol::TCP.covers(L4Protocol::TCP));
        assert!(!L4Protocol::TCP.covers(L4Protocol::UDP));
    }

    #[test]
    fn port_protocol_zero_port_means_any_port() {
        let (_c, _t) = cilium_test_ctx!("pkg/policy/api/ports.go", "PortProtocol", "tenant-pol-pp-zero");
        let pp = PortProtocol::new(0, L4Protocol::TCP);
        assert!(pp.covers(80, L4Protocol::TCP));
        assert!(pp.covers(443, L4Protocol::TCP));
        assert!(!pp.covers(80, L4Protocol::UDP));
    }

    #[test]
    fn port_protocol_specific_port_covers_only_that_port() {
        let (_c, _t) = cilium_test_ctx!("pkg/policy/api/ports.go", "PortProtocol", "tenant-pol-pp-spec");
        let pp = PortProtocol::new(80, L4Protocol::TCP);
        assert!(pp.covers(80, L4Protocol::TCP));
        assert!(!pp.covers(443, L4Protocol::TCP));
    }

    // ── ICMP rules ───────────────────────────────────────────────────────────

    #[test]
    fn icmp_rule_v4_matches_correct_type() {
        let (_c, _t) = cilium_test_ctx!("pkg/policy/api/icmp.go", "ICMPField.Match", "tenant-pol-icmp-v4");
        let r = IcmpRule { family: IcmpFamily::V4, icmp_type: 8 /* echo request */ };
        assert!(r.matches(IcmpFamily::V4, 8));
        assert!(!r.matches(IcmpFamily::V4, 0));
    }

    #[test]
    fn icmp_rule_v6_matches_correct_type() {
        let (_c, _t) = cilium_test_ctx!("pkg/policy/api/icmp.go", "ICMPField.Match", "tenant-pol-icmp-v6");
        let r = IcmpRule { family: IcmpFamily::V6, icmp_type: 128 };
        assert!(r.matches(IcmpFamily::V6, 128));
    }

    #[test]
    fn icmp_rule_family_mismatch_does_not_match() {
        let (_c, _t) = cilium_test_ctx!("pkg/policy/api/icmp.go", "ICMPField.Match", "tenant-pol-icmp-fam");
        let r = IcmpRule { family: IcmpFamily::V4, icmp_type: 8 };
        assert!(!r.matches(IcmpFamily::V6, 8));
    }

    // ── Repository ───────────────────────────────────────────────────────────

    #[test]
    fn repository_add_and_remove() {
        let (_c, _t) = cilium_test_ctx!("pkg/policy/repository.go", "PolicyRepository", "tenant-pol-repo");
        let mut repo = PolicyRepository::new();
        let rule = Rule::new("allow-all", TenantId::new("tenant-pol-repo").expect("test fixture"), EndpointSelector::empty());
        repo.add(rule);
        assert_eq!(repo.len(), 1);
        assert!(repo.remove("allow-all"));
        assert!(repo.is_empty());
    }

    #[test]
    fn repository_remove_unknown_returns_false() {
        let (_c, _t) = cilium_test_ctx!("pkg/policy/repository.go", "PolicyRepository.Remove", "tenant-pol-rmunk");
        let mut repo = PolicyRepository::new();
        assert!(!repo.remove("nope"));
    }

    // ── PolicyMap lookup ─────────────────────────────────────────────────────

    #[test]
    fn policy_map_exact_lookup_returns_allow() {
        let (_c, _t) = cilium_test_ctx!("pkg/maps/policymap/policymap.go", "PolicyMap.Lookup", "tenant-pol-pm-allow");
        let mut pm = PolicyMap::new();
        pm.ingress_enforced = true;
        pm.allow(PolicyKey { peer_identity: 256, port: 80, protocol: L4Protocol::TCP, direction: Direction::Ingress }, None);
        let v = pm.lookup(256, 80, L4Protocol::TCP, Direction::Ingress);
        assert_eq!(v.verdict, Verdict::Allow);
    }

    #[test]
    fn policy_map_default_deny_when_enforced() {
        let (_c, _t) = cilium_test_ctx!("pkg/maps/policymap/policymap.go", "PolicyMap.Lookup", "tenant-pol-pm-deny");
        let mut pm = PolicyMap::new();
        pm.ingress_enforced = true;
        let v = pm.lookup(256, 80, L4Protocol::TCP, Direction::Ingress);
        assert_eq!(v.verdict, Verdict::Deny);
    }

    #[test]
    fn policy_map_default_allow_when_not_enforced() {
        let (_c, _t) = cilium_test_ctx!("pkg/maps/policymap/policymap.go", "PolicyMap.Lookup", "tenant-pol-pm-noenf");
        let pm = PolicyMap::new();
        let v = pm.lookup(256, 80, L4Protocol::TCP, Direction::Ingress);
        assert_eq!(v.verdict, Verdict::Allow);
    }

    #[test]
    fn policy_map_wildcard_port_matches_specific_request() {
        let (_c, _t) = cilium_test_ctx!("pkg/maps/policymap/policymap.go", "PolicyMap.WildcardPort", "tenant-pol-pm-wcp");
        let mut pm = PolicyMap::new();
        pm.ingress_enforced = true;
        pm.allow(PolicyKey { peer_identity: 256, port: 0, protocol: L4Protocol::TCP, direction: Direction::Ingress }, None);
        let v = pm.lookup(256, 8080, L4Protocol::TCP, Direction::Ingress);
        assert_eq!(v.verdict, Verdict::Allow);
    }

    #[test]
    fn policy_map_any_proto_wildcard_covers_tcp_and_udp() {
        let (_c, _t) = cilium_test_ctx!("pkg/maps/policymap/policymap.go", "PolicyMap.WildcardProto", "tenant-pol-pm-wcproto");
        let mut pm = PolicyMap::new();
        pm.ingress_enforced = true;
        pm.allow(PolicyKey { peer_identity: 256, port: 0, protocol: L4Protocol::Any, direction: Direction::Ingress }, None);
        assert_eq!(pm.lookup(256, 80, L4Protocol::TCP, Direction::Ingress).verdict, Verdict::Allow);
        assert_eq!(pm.lookup(256, 53, L4Protocol::UDP, Direction::Ingress).verdict, Verdict::Allow);
    }

    #[test]
    fn policy_map_l7_redirect_port_returned_on_lookup() {
        let (_c, _t) = cilium_test_ctx!("pkg/maps/policymap/policymap.go", "MapStateEntry.proxyPort", "tenant-pol-pm-l7");
        let mut pm = PolicyMap::new();
        pm.ingress_enforced = true;
        pm.allow(PolicyKey { peer_identity: 256, port: 80, protocol: L4Protocol::TCP, direction: Direction::Ingress }, Some(15001));
        let v = pm.lookup(256, 80, L4Protocol::TCP, Direction::Ingress);
        assert_eq!(v.l7_redirect_port, Some(15001));
    }

    #[test]
    fn policy_map_id_all_fallback_allow() {
        let (_c, _t) = cilium_test_ctx!("pkg/maps/policymap/policymap.go", "PolicyMap.WildcardIdentity", "tenant-pol-pm-all");
        let mut pm = PolicyMap::new();
        pm.ingress_enforced = true;
        pm.allow(PolicyKey { peer_identity: ID_ALL, port: 0, protocol: L4Protocol::Any, direction: Direction::Ingress }, None);
        // Any peer should be allowed via the all-fallback.
        assert_eq!(pm.lookup(999, 80, L4Protocol::TCP, Direction::Ingress).verdict, Verdict::Allow);
    }

    // ── Distillery ───────────────────────────────────────────────────────────

    fn make_resolver(entries: &[(u32, &[(&str, &str)])]) -> InMemoryIdentityResolver {
        let mut r = InMemoryIdentityResolver::new();
        for (id, lp) in entries {
            r.insert(*id, ls(lp));
        }
        r
    }

    #[test]
    fn distill_no_rules_no_enforcement_default_allow() {
        let (_c, tenant) = cilium_test_ctx!("pkg/policy/distillery.go", "distillPolicy", "tenant-pol-dist-empty");
        let repo = PolicyRepository::new();
        let resolver = InMemoryIdentityResolver::new();
        let map = distill(&repo, &tenant, &ls(&[("app", "web")]), PolicyEnforcementMode::Default, &resolver).unwrap();
        assert!(!map.ingress_enforced);
        assert!(!map.egress_enforced);
        let v = map.lookup(256, 80, L4Protocol::TCP, Direction::Ingress);
        assert_eq!(v.verdict, Verdict::Allow);
    }

    #[test]
    fn distill_ingress_rule_enables_default_deny() {
        let (_c, tenant) = cilium_test_ctx!("pkg/policy/distillery.go", "distillPolicy.IngressEnforced", "tenant-pol-dist-deny");
        let mut repo = PolicyRepository::new();
        let mut rule = Rule::new("r1", tenant.clone(), endpoint_sel(&[("app", "web")]));
        rule.ingress.push(IngressRule {
            from_endpoints: vec![endpoint_sel(&[("app", "client")])],
            ..Default::default()
        });
        repo.add(rule);
        let resolver = make_resolver(&[(MIN_LOCAL_IDENTITY, &[("app", "client")])]);
        let map = distill(&repo, &tenant, &ls(&[("app", "web")]), PolicyEnforcementMode::Default, &resolver).unwrap();
        assert!(map.ingress_enforced);
        // client is allowed.
        assert_eq!(map.lookup(MIN_LOCAL_IDENTITY, 80, L4Protocol::TCP, Direction::Ingress).verdict, Verdict::Allow);
        // unknown peer is denied.
        assert_eq!(map.lookup(999, 80, L4Protocol::TCP, Direction::Ingress).verdict, Verdict::Deny);
    }

    #[test]
    fn distill_ingress_from_entity_world_creates_world_entry() {
        let (_c, tenant) = cilium_test_ctx!("pkg/policy/distillery.go", "distillPolicy.FromEntities", "tenant-pol-dist-world");
        let mut repo = PolicyRepository::new();
        let mut rule = Rule::new("allow-world", tenant.clone(), endpoint_sel(&[("app", "ingest")]));
        rule.ingress.push(IngressRule {
            from_entities: vec![Entity::World],
            to_ports: vec![PortRule { ports: vec![PortProtocol::new(443, L4Protocol::TCP)], l7_redirect_port: None }],
            ..Default::default()
        });
        repo.add(rule);
        let resolver = InMemoryIdentityResolver::new();
        let map = distill(&repo, &tenant, &ls(&[("app", "ingest")]), PolicyEnforcementMode::Default, &resolver).unwrap();
        let v = map.lookup(ID_WORLD, 443, L4Protocol::TCP, Direction::Ingress);
        assert_eq!(v.verdict, Verdict::Allow);
        // Other ports remain denied.
        assert_eq!(map.lookup(ID_WORLD, 80, L4Protocol::TCP, Direction::Ingress).verdict, Verdict::Deny);
    }

    #[test]
    fn distill_ingress_from_cidr_emits_world_peer() {
        let (_c, tenant) = cilium_test_ctx!("pkg/policy/distillery.go", "distillPolicy.FromCIDR", "tenant-pol-dist-cidr");
        let mut repo = PolicyRepository::new();
        let mut rule = Rule::new("from-cidr", tenant.clone(), endpoint_sel(&[("app", "edge")]));
        rule.ingress.push(IngressRule {
            from_cidr: vec![CidrRule::new("198.51.100.0/24")],
            to_ports: vec![PortRule { ports: vec![PortProtocol::new(443, L4Protocol::TCP)], l7_redirect_port: None }],
            ..Default::default()
        });
        repo.add(rule);
        let resolver = InMemoryIdentityResolver::new();
        let map = distill(&repo, &tenant, &ls(&[("app", "edge")]), PolicyEnforcementMode::Default, &resolver).unwrap();
        assert_eq!(map.lookup(ID_WORLD, 443, L4Protocol::TCP, Direction::Ingress).verdict, Verdict::Allow);
    }

    #[test]
    fn distill_egress_to_endpoint_creates_egress_allow() {
        let (_c, tenant) = cilium_test_ctx!("pkg/policy/distillery.go", "distillPolicy.Egress", "tenant-pol-dist-eg");
        let mut repo = PolicyRepository::new();
        let mut rule = Rule::new("eg-to-db", tenant.clone(), endpoint_sel(&[("app", "api")]));
        rule.egress.push(EgressRule {
            to_endpoints: vec![endpoint_sel(&[("app", "db")])],
            to_ports: vec![PortRule { ports: vec![PortProtocol::new(5432, L4Protocol::TCP)], l7_redirect_port: None }],
            ..Default::default()
        });
        repo.add(rule);
        let resolver = make_resolver(&[(MIN_LOCAL_IDENTITY + 7, &[("app", "db")])]);
        let map = distill(&repo, &tenant, &ls(&[("app", "api")]), PolicyEnforcementMode::Default, &resolver).unwrap();
        assert!(map.egress_enforced);
        assert_eq!(
            map.lookup(MIN_LOCAL_IDENTITY + 7, 5432, L4Protocol::TCP, Direction::Egress).verdict,
            Verdict::Allow
        );
    }

    #[test]
    fn distill_endpoint_selector_no_match_skips_rule() {
        let (_c, tenant) = cilium_test_ctx!("pkg/policy/distillery.go", "distillPolicy.SkipNonMatching", "tenant-pol-dist-skip");
        let mut repo = PolicyRepository::new();
        let mut rule = Rule::new("for-other-app", tenant.clone(), endpoint_sel(&[("app", "other")]));
        rule.ingress.push(IngressRule {
            from_endpoints: vec![endpoint_sel(&[("app", "client")])],
            ..Default::default()
        });
        repo.add(rule);
        let resolver = make_resolver(&[(MIN_LOCAL_IDENTITY, &[("app", "client")])]);
        let map = distill(&repo, &tenant, &ls(&[("app", "web")]), PolicyEnforcementMode::Default, &resolver).unwrap();
        assert!(!map.ingress_enforced);
        assert!(map.entries.is_empty());
    }

    #[test]
    fn distill_combines_multiple_rules_for_same_endpoint() {
        let (_c, tenant) = cilium_test_ctx!("pkg/policy/distillery.go", "distillPolicy.Combine", "tenant-pol-dist-comb");
        let mut repo = PolicyRepository::new();
        let mut r1 = Rule::new("r1", tenant.clone(), endpoint_sel(&[("app", "web")]));
        r1.ingress.push(IngressRule {
            from_endpoints: vec![endpoint_sel(&[("app", "client")])],
            to_ports: vec![PortRule { ports: vec![PortProtocol::new(80, L4Protocol::TCP)], l7_redirect_port: None }],
            ..Default::default()
        });
        let mut r2 = Rule::new("r2", tenant.clone(), endpoint_sel(&[("app", "web")]));
        r2.ingress.push(IngressRule {
            from_endpoints: vec![endpoint_sel(&[("app", "metrics")])],
            to_ports: vec![PortRule { ports: vec![PortProtocol::new(9090, L4Protocol::TCP)], l7_redirect_port: None }],
            ..Default::default()
        });
        repo.add(r1);
        repo.add(r2);
        let resolver = make_resolver(&[
            (MIN_LOCAL_IDENTITY, &[("app", "client")]),
            (MIN_LOCAL_IDENTITY + 1, &[("app", "metrics")]),
        ]);
        let map = distill(&repo, &tenant, &ls(&[("app", "web")]), PolicyEnforcementMode::Default, &resolver).unwrap();
        assert_eq!(map.lookup(MIN_LOCAL_IDENTITY, 80, L4Protocol::TCP, Direction::Ingress).verdict, Verdict::Allow);
        assert_eq!(map.lookup(MIN_LOCAL_IDENTITY + 1, 9090, L4Protocol::TCP, Direction::Ingress).verdict, Verdict::Allow);
        // Cross-port denied.
        assert_eq!(map.lookup(MIN_LOCAL_IDENTITY, 9090, L4Protocol::TCP, Direction::Ingress).verdict, Verdict::Deny);
    }

    #[test]
    fn distill_l7_redirect_port_propagates_to_map() {
        let (_c, tenant) = cilium_test_ctx!("pkg/policy/distillery.go", "distillPolicy.L7", "tenant-pol-dist-l7");
        let mut repo = PolicyRepository::new();
        let mut rule = Rule::new("l7", tenant.clone(), endpoint_sel(&[("app", "web")]));
        rule.ingress.push(IngressRule {
            from_endpoints: vec![endpoint_sel(&[("app", "client")])],
            to_ports: vec![PortRule {
                ports: vec![PortProtocol::new(80, L4Protocol::TCP)],
                l7_redirect_port: Some(10042),
            }],
            ..Default::default()
        });
        repo.add(rule);
        let resolver = make_resolver(&[(MIN_LOCAL_IDENTITY, &[("app", "client")])]);
        let map = distill(&repo, &tenant, &ls(&[("app", "web")]), PolicyEnforcementMode::Default, &resolver).unwrap();
        let v = map.lookup(MIN_LOCAL_IDENTITY, 80, L4Protocol::TCP, Direction::Ingress);
        assert_eq!(v.verdict, Verdict::Allow);
        assert_eq!(v.l7_redirect_port, Some(10042));
    }

    #[test]
    fn distill_rejects_cross_tenant_rule() {
        let other = TenantId::new("tenant-pol-dist-other").expect("test fixture");
        let (_c, mine) = cilium_test_ctx!("pkg/policy/distillery.go", "distillPolicy.TenantIsolation", "tenant-pol-dist-mine");
        let mut repo = PolicyRepository::new();
        repo.add(Rule::new("foreign", other, EndpointSelector::empty()));
        let resolver = InMemoryIdentityResolver::new();
        let err = distill(&repo, &mine, &ls(&[("app", "x")]), PolicyEnforcementMode::Default, &resolver).unwrap_err();
        assert!(matches!(err, PolicyError::TenantDenied { .. }));
    }

    #[test]
    fn distill_rule_with_no_ports_is_l3_only_allow() {
        let (_c, tenant) = cilium_test_ctx!("pkg/policy/distillery.go", "distillPolicy.L3Only", "tenant-pol-dist-l3");
        let mut repo = PolicyRepository::new();
        let mut rule = Rule::new("l3-only", tenant.clone(), endpoint_sel(&[("app", "web")]));
        rule.ingress.push(IngressRule {
            from_endpoints: vec![endpoint_sel(&[("app", "client")])],
            ..Default::default()
        });
        repo.add(rule);
        let resolver = make_resolver(&[(MIN_LOCAL_IDENTITY, &[("app", "client")])]);
        let map = distill(&repo, &tenant, &ls(&[("app", "web")]), PolicyEnforcementMode::Default, &resolver).unwrap();
        // Any port is allowed for the client.
        assert_eq!(map.lookup(MIN_LOCAL_IDENTITY, 80, L4Protocol::TCP, Direction::Ingress).verdict, Verdict::Allow);
        assert_eq!(map.lookup(MIN_LOCAL_IDENTITY, 9999, L4Protocol::UDP, Direction::Ingress).verdict, Verdict::Allow);
    }

    #[test]
    fn distill_entity_cluster_expands_to_known_identities() {
        let (_c, tenant) = cilium_test_ctx!("pkg/policy/distillery.go", "distillPolicy.EntityCluster", "tenant-pol-dist-cl");
        let mut repo = PolicyRepository::new();
        let mut rule = Rule::new("from-cluster", tenant.clone(), endpoint_sel(&[("app", "web")]));
        rule.ingress.push(IngressRule {
            from_entities: vec![Entity::Cluster],
            ..Default::default()
        });
        repo.add(rule);
        let resolver = make_resolver(&[
            (MIN_LOCAL_IDENTITY, &[("app", "a")]),
            (MIN_LOCAL_IDENTITY + 1, &[("app", "b")]),
        ]);
        let map = distill(&repo, &tenant, &ls(&[("app", "web")]), PolicyEnforcementMode::Default, &resolver).unwrap();
        assert!(map.ingress_enforced);
        assert_eq!(map.lookup(MIN_LOCAL_IDENTITY, 80, L4Protocol::TCP, Direction::Ingress).verdict, Verdict::Allow);
        assert_eq!(map.lookup(MIN_LOCAL_IDENTITY + 1, 80, L4Protocol::TCP, Direction::Ingress).verdict, Verdict::Allow);
        // World remains denied (Cluster ≠ World).
        assert_eq!(map.lookup(ID_WORLD, 80, L4Protocol::TCP, Direction::Ingress).verdict, Verdict::Deny);
    }

    #[test]
    fn distill_from_requires_filters_peers() {
        let (_c, tenant) = cilium_test_ctx!("pkg/policy/distillery.go", "distillPolicy.FromRequires", "tenant-pol-dist-req");
        let mut repo = PolicyRepository::new();
        let mut rule = Rule::new("strict", tenant.clone(), endpoint_sel(&[("app", "web")]));
        rule.ingress.push(IngressRule {
            from_endpoints: vec![endpoint_sel(&[("app", "client")])],
            from_requires: vec![endpoint_sel(&[("env", "prod")])],
            ..Default::default()
        });
        repo.add(rule);
        let resolver = make_resolver(&[
            (MIN_LOCAL_IDENTITY, &[("app", "client"), ("env", "prod")]),
            (MIN_LOCAL_IDENTITY + 1, &[("app", "client"), ("env", "dev")]),
        ]);
        let map = distill(&repo, &tenant, &ls(&[("app", "web")]), PolicyEnforcementMode::Default, &resolver).unwrap();
        assert_eq!(map.lookup(MIN_LOCAL_IDENTITY, 80, L4Protocol::TCP, Direction::Ingress).verdict, Verdict::Allow);
        assert_eq!(map.lookup(MIN_LOCAL_IDENTITY + 1, 80, L4Protocol::TCP, Direction::Ingress).verdict, Verdict::Deny);
    }

    #[test]
    fn distill_icmp_rule_emits_icmp_entry() {
        let (_c, tenant) = cilium_test_ctx!("pkg/policy/distillery.go", "distillPolicy.ICMP", "tenant-pol-dist-icmp");
        let mut repo = PolicyRepository::new();
        let mut rule = Rule::new("ping-allowed", tenant.clone(), endpoint_sel(&[("app", "web")]));
        rule.ingress.push(IngressRule {
            from_endpoints: vec![endpoint_sel(&[("app", "client")])],
            icmps: vec![IcmpRule { family: IcmpFamily::V4, icmp_type: 8 }],
            ..Default::default()
        });
        repo.add(rule);
        let resolver = make_resolver(&[(MIN_LOCAL_IDENTITY, &[("app", "client")])]);
        let map = distill(&repo, &tenant, &ls(&[("app", "web")]), PolicyEnforcementMode::Default, &resolver).unwrap();
        // ICMP type 8 → key { port=8, proto=ICMP }.
        let v = map.lookup(MIN_LOCAL_IDENTITY, 8, L4Protocol::ICMP, Direction::Ingress);
        assert_eq!(v.verdict, Verdict::Allow);
        // Other ICMP type denied.
        assert_eq!(
            map.lookup(MIN_LOCAL_IDENTITY, 0, L4Protocol::ICMP, Direction::Ingress).verdict,
            Verdict::Deny
        );
    }

    #[test]
    fn distill_enforcement_always_default_denies_everything() {
        let (_c, tenant) = cilium_test_ctx!("pkg/policy/policy.go", "PolicyEnforcement.Always", "tenant-pol-dist-always");
        let repo = PolicyRepository::new();
        let resolver = InMemoryIdentityResolver::new();
        let map = distill(&repo, &tenant, &ls(&[("app", "web")]), PolicyEnforcementMode::Always, &resolver).unwrap();
        assert!(map.ingress_enforced);
        assert!(map.egress_enforced);
        assert_eq!(map.lookup(123, 80, L4Protocol::TCP, Direction::Ingress).verdict, Verdict::Deny);
        assert_eq!(map.lookup(123, 80, L4Protocol::TCP, Direction::Egress).verdict, Verdict::Deny);
    }

    #[test]
    fn distill_enforcement_never_allows_everything_no_entries() {
        let (_c, tenant) = cilium_test_ctx!("pkg/policy/policy.go", "PolicyEnforcement.Never", "tenant-pol-dist-never");
        let mut repo = PolicyRepository::new();
        let mut rule = Rule::new("ignored", tenant.clone(), EndpointSelector::empty());
        rule.ingress.push(IngressRule { from_entities: vec![Entity::Host], ..Default::default() });
        repo.add(rule);
        let resolver = InMemoryIdentityResolver::new();
        let map = distill(&repo, &tenant, &ls(&[("app", "web")]), PolicyEnforcementMode::Never, &resolver).unwrap();
        assert!(!map.ingress_enforced);
        assert!(!map.egress_enforced);
        assert!(map.entries.is_empty());
        assert_eq!(map.lookup(99, 80, L4Protocol::TCP, Direction::Ingress).verdict, Verdict::Allow);
    }

    #[test]
    fn distill_endpoint_with_two_directions_enforces_both() {
        let (_c, tenant) = cilium_test_ctx!("pkg/policy/distillery.go", "distillPolicy.BothDirections", "tenant-pol-dist-both");
        let mut repo = PolicyRepository::new();
        let mut rule = Rule::new("both", tenant.clone(), endpoint_sel(&[("app", "api")]));
        rule.ingress.push(IngressRule { from_entities: vec![Entity::Host], ..Default::default() });
        rule.egress.push(EgressRule { to_entities: vec![Entity::World], ..Default::default() });
        repo.add(rule);
        let resolver = InMemoryIdentityResolver::new();
        let map = distill(&repo, &tenant, &ls(&[("app", "api")]), PolicyEnforcementMode::Default, &resolver).unwrap();
        assert!(map.ingress_enforced);
        assert!(map.egress_enforced);
        assert_eq!(map.lookup(ID_HOST, 0, L4Protocol::Any, Direction::Ingress).verdict, Verdict::Allow);
        assert_eq!(map.lookup(ID_WORLD, 0, L4Protocol::Any, Direction::Egress).verdict, Verdict::Allow);
        // Reverse directions are still denied.
        assert_eq!(map.lookup(ID_HOST, 0, L4Protocol::Any, Direction::Egress).verdict, Verdict::Deny);
    }

    #[test]
    fn distill_rule_with_multiple_port_protos_emits_each_entry() {
        let (_c, tenant) = cilium_test_ctx!("pkg/policy/distillery.go", "distillPolicy.MultiPort", "tenant-pol-dist-mp");
        let mut repo = PolicyRepository::new();
        let mut rule = Rule::new("dual", tenant.clone(), endpoint_sel(&[("app", "x")]));
        rule.ingress.push(IngressRule {
            from_endpoints: vec![endpoint_sel(&[("app", "y")])],
            to_ports: vec![PortRule {
                ports: vec![
                    PortProtocol::new(80, L4Protocol::TCP),
                    PortProtocol::new(53, L4Protocol::UDP),
                ],
                l7_redirect_port: None,
            }],
            ..Default::default()
        });
        repo.add(rule);
        let resolver = make_resolver(&[(MIN_LOCAL_IDENTITY, &[("app", "y")])]);
        let map = distill(&repo, &tenant, &ls(&[("app", "x")]), PolicyEnforcementMode::Default, &resolver).unwrap();
        assert_eq!(map.lookup(MIN_LOCAL_IDENTITY, 80, L4Protocol::TCP, Direction::Ingress).verdict, Verdict::Allow);
        assert_eq!(map.lookup(MIN_LOCAL_IDENTITY, 53, L4Protocol::UDP, Direction::Ingress).verdict, Verdict::Allow);
        // 80/UDP is denied (different proto).
        assert_eq!(map.lookup(MIN_LOCAL_IDENTITY, 80, L4Protocol::UDP, Direction::Ingress).verdict, Verdict::Deny);
    }

    #[test]
    fn distill_to_ports_empty_block_means_any_port() {
        let (_c, tenant) = cilium_test_ctx!("pkg/policy/distillery.go", "distillPolicy.EmptyToPorts", "tenant-pol-dist-empports");
        let mut repo = PolicyRepository::new();
        let mut rule = Rule::new("any-port", tenant.clone(), endpoint_sel(&[("app", "x")]));
        rule.ingress.push(IngressRule {
            from_endpoints: vec![endpoint_sel(&[("app", "y")])],
            to_ports: vec![PortRule { ports: vec![], l7_redirect_port: None }],
            ..Default::default()
        });
        repo.add(rule);
        let resolver = make_resolver(&[(MIN_LOCAL_IDENTITY, &[("app", "y")])]);
        let map = distill(&repo, &tenant, &ls(&[("app", "x")]), PolicyEnforcementMode::Default, &resolver).unwrap();
        assert_eq!(map.lookup(MIN_LOCAL_IDENTITY, 1234, L4Protocol::TCP, Direction::Ingress).verdict, Verdict::Allow);
    }
}

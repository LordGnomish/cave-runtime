// SPDX-License-Identifier: AGPL-3.0-or-later
//! Cloud node controller.
//!
//! Mirrors `cloud-provider/controllers/node/node_controller.go` and the
//! `nodelifecycle` controller that complements it. On node join, the
//! controller:
//!
//! 1. Sets `node.spec.providerID`.
//! 2. Adds standard topology labels (`topology.kubernetes.io/zone`,
//!    `topology.kubernetes.io/region`, `node.kubernetes.io/instance-type`).
//! 3. Sync node addresses (`InternalIP`, `ExternalIP`, `Hostname`, …).
//! 4. Removes the
//!    `node.cloudprovider.kubernetes.io/uninitialized:NoSchedule`
//!    *initializer taint* once the cloud has finished annotating the node.
//! 5. Applies / removes the `node.kubernetes.io/unreachable`,
//!    `node.cloudprovider.kubernetes.io/shutdown` and
//!    `node.kubernetes.io/out-of-service` taints based on the cloud's view of
//!    the underlying instance.
//! 6. Deletes the Node when the cloud reports the instance as
//!    `Terminated` / `NotFound` (the "shutdown deletion" pathway upstream).

use crate::types::{Cite, CloudError, Reconcile, TenantId};
use serde::{Deserialize, Serialize};

// ─── Standard label keys (verbatim from upstream `well_known_labels.go`) ─────

pub const LABEL_ZONE: &str = "topology.kubernetes.io/zone";
pub const LABEL_REGION: &str = "topology.kubernetes.io/region";
pub const LABEL_INSTANCE_TYPE: &str = "node.kubernetes.io/instance-type";
pub const LABEL_HOSTNAME: &str = "kubernetes.io/hostname";

// ─── Standard taint keys (from `well_known_taints.go`) ───────────────────────

/// Initializer taint set by kubelet, removed by this controller after the
/// cloud-provider has finished annotating the node.
pub const INITIALIZER_TAINT_KEY: &str = "node.cloudprovider.kubernetes.io/uninitialized";
/// Set when cloud reports the instance as `Shutdown` (powered off but not
/// deleted). Drains pods from the node.
pub const SHUTDOWN_TAINT_KEY: &str = "node.cloudprovider.kubernetes.io/shutdown";
/// Set when the cloud SDK is unreachable; same key as the kubelet
/// node-condition taint so existing tooling continues to work.
pub const UNREACHABLE_TAINT_KEY: &str = "node.kubernetes.io/unreachable";
/// Set when the cloud reports the instance as in maintenance / out of service.
/// kube-controller-manager's nodelifecycle reads this and triggers volume
/// detach.
pub const OUT_OF_SERVICE_TAINT_KEY: &str = "node.kubernetes.io/out-of-service";
/// Set when a node is in NotReady condition.
pub const NOT_READY_TAINT_KEY: &str = "node.kubernetes.io/not-ready";

// ─── Address types ───────────────────────────────────────────────────────────

/// Mirrors `core/v1.NodeAddressType`. Order in this enum is the canonical
/// kubelet preference order — first hit wins when a consumer wants "the"
/// address for a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NodeAddressType {
    InternalIP,
    ExternalIP,
    InternalDNS,
    ExternalDNS,
    Hostname,
}

impl NodeAddressType {
    pub const fn key(self) -> &'static str {
        match self {
            NodeAddressType::InternalIP => "InternalIP",
            NodeAddressType::ExternalIP => "ExternalIP",
            NodeAddressType::InternalDNS => "InternalDNS",
            NodeAddressType::ExternalDNS => "ExternalDNS",
            NodeAddressType::Hostname => "Hostname",
        }
    }

    /// Canonical preference order — kubelet picks the first match per
    /// `--node-status-update-frequency` round.
    pub const fn precedence(self) -> u8 {
        match self {
            NodeAddressType::InternalIP => 0,
            NodeAddressType::ExternalIP => 1,
            NodeAddressType::InternalDNS => 2,
            NodeAddressType::ExternalDNS => 3,
            NodeAddressType::Hostname => 4,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeAddress {
    pub kind: NodeAddressType,
    pub address: String,
}

impl NodeAddress {
    pub fn new(kind: NodeAddressType, address: impl Into<String>) -> Self {
        Self { kind, address: address.into() }
    }
}

// ─── Cloud-side instance state ───────────────────────────────────────────────

/// Mirrors `cloudprovider.InstanceMetadata.Status` and the lifecycle states in
/// `nodelifecyclecontroller`. `NotFound` means the cloud SDK returned 404 —
/// the corresponding Node must be deleted, mirroring upstream
/// `ensureNodeExistsByProviderID` behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum InstanceState {
    Running,
    Shutdown,
    Terminated,
    NotFound,
    /// Provider returned a transient error (HTTP 5xx, network blip, etc.).
    Unreachable,
}

impl InstanceState {
    /// True iff the instance is gone for good — the Node should be deleted.
    pub const fn requires_deletion(self) -> bool {
        matches!(self, InstanceState::Terminated | InstanceState::NotFound)
    }

    /// True iff the instance is alive and reachable; clears failure taints.
    pub const fn is_healthy(self) -> bool {
        matches!(self, InstanceState::Running)
    }

    /// Taint key that should be applied for this state. `None` for healthy
    /// instances and instances that require deletion (those bypass tainting
    /// and go straight to delete).
    pub const fn failure_taint(self) -> Option<&'static str> {
        match self {
            InstanceState::Shutdown => Some(SHUTDOWN_TAINT_KEY),
            InstanceState::Unreachable => Some(UNREACHABLE_TAINT_KEY),
            InstanceState::Running | InstanceState::Terminated | InstanceState::NotFound => None,
        }
    }
}

// ─── Node + cloud views ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeView {
    pub name: String,
    pub provider_id: Option<String>,
    pub zone: Option<String>,
    pub region: Option<String>,
    pub instance_type: Option<String>,
    pub initializer_taint_present: bool,
    /// Currently-applied set of cloud-failure taints (excluding the
    /// initializer taint above, which has its own boolean for symmetry with
    /// the upstream initial design).
    #[serde(default)]
    pub cloud_taints: Vec<String>,
    /// Currently-published `Node.status.addresses`.
    #[serde(default)]
    pub addresses: Vec<NodeAddress>,
}

impl NodeView {
    /// Construct a fresh, freshly-joined node — no annotations yet, taint
    /// present. Mirrors what kubelet emits on first registration.
    pub fn fresh(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            provider_id: None,
            zone: None,
            region: None,
            instance_type: None,
            initializer_taint_present: true,
            cloud_taints: Vec::new(),
            addresses: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudFacts {
    pub provider_id: String,
    pub zone: String,
    pub region: String,
    pub instance_type: String,
    /// Addresses the cloud is publishing for this node. Empty on a stub
    /// provider; populated from `InstanceMetadata.NodeAddresses` upstream.
    #[serde(default)]
    pub addresses: Vec<NodeAddress>,
}

impl CloudFacts {
    pub fn minimal(
        provider_id: impl Into<String>,
        zone: impl Into<String>,
        region: impl Into<String>,
        instance_type: impl Into<String>,
    ) -> Self {
        Self {
            provider_id: provider_id.into(),
            zone: zone.into(),
            region: region.into(),
            instance_type: instance_type.into(),
            addresses: Vec::new(),
        }
    }
}

// ─── Provider-id parsing ─────────────────────────────────────────────────────

/// Split a `<scheme>://<id>` provider-id into its parts, or `None` if it
/// does not match the upstream shape. Mirrors `getInstanceProviderID`.
pub fn parse_provider_id(s: &str) -> Option<(&str, &str)> {
    let (scheme, rest) = s.split_once("://")?;
    if scheme.is_empty() || rest.is_empty() {
        return None;
    }
    Some((scheme, rest))
}

/// Convenience — return just the scheme (e.g. `"hcloud"`, `"azure"`).
pub fn provider_id_scheme(s: &str) -> Option<&str> {
    parse_provider_id(s).map(|(scheme, _)| scheme)
}

/// Build a canonical provider-id from parts. Stable wrapper so callers don't
/// hard-code `"://"`.
pub fn format_provider_id(scheme: &str, id: &str) -> String {
    format!("{scheme}://{id}")
}

// ─── Initialization ──────────────────────────────────────────────────────────

/// True iff every standard label has been written. Address sync is *not*
/// required for initialization — kubelet may publish addresses before or
/// after the cloud annotations land.
pub fn is_initialised(node: &NodeView) -> bool {
    node.provider_id.is_some()
        && node.zone.is_some()
        && node.region.is_some()
        && node.instance_type.is_some()
}

/// Number of topology labels still drifting from cloud facts.
pub fn label_drift(node: &NodeView, facts: &CloudFacts) -> u32 {
    let mut drift = 0;
    if node.provider_id.as_deref() != Some(facts.provider_id.as_str()) {
        drift += 1;
    }
    if node.zone.as_deref() != Some(facts.zone.as_str()) {
        drift += 1;
    }
    if node.region.as_deref() != Some(facts.region.as_str()) {
        drift += 1;
    }
    if node.instance_type.as_deref() != Some(facts.instance_type.as_str()) {
        drift += 1;
    }
    drift
}

// ─── Address sync ────────────────────────────────────────────────────────────

/// Canonicalize a list of addresses: dedupe (kind, address) pairs, sort by
/// kubelet precedence, then by address for deterministic ordering. Mirrors
/// `mergeNodeAddresses` upstream.
pub fn canonicalize_addresses(input: &[NodeAddress]) -> Vec<NodeAddress> {
    let mut seen: Vec<NodeAddress> = Vec::with_capacity(input.len());
    for a in input {
        if !seen.iter().any(|s| s.kind == a.kind && s.address == a.address) {
            seen.push(a.clone());
        }
    }
    seen.sort_by(|a, b| {
        a.kind
            .precedence()
            .cmp(&b.kind.precedence())
            .then_with(|| a.address.cmp(&b.address))
    });
    seen
}

/// Difference between a node's published addresses and the cloud's view. The
/// (added, removed) pair captures what the controller would PATCH.
pub fn address_diff(
    node: &[NodeAddress],
    cloud: &[NodeAddress],
) -> (Vec<NodeAddress>, Vec<NodeAddress>) {
    let want = canonicalize_addresses(cloud);
    let have = canonicalize_addresses(node);
    let added: Vec<_> = want
        .iter()
        .filter(|w| !have.iter().any(|h| h.kind == w.kind && h.address == w.address))
        .cloned()
        .collect();
    let removed: Vec<_> = have
        .iter()
        .filter(|h| !want.iter().any(|w| w.kind == h.kind && w.address == h.address))
        .cloned()
        .collect();
    (added, removed)
}

/// Pick the kubelet-preferred address — first match in canonical order.
/// Mirrors `getNodeAddressesFromNodeIP` selection logic.
pub fn preferred_address(addrs: &[NodeAddress]) -> Option<&NodeAddress> {
    canonicalize_addresses(addrs).into_iter().next().and_then(|first| {
        addrs.iter().find(|a| a.kind == first.kind && a.address == first.address)
    })
}

// ─── Taint helpers ───────────────────────────────────────────────────────────

/// True iff `key` is present in `node.cloud_taints`.
pub fn has_taint(node: &NodeView, key: &str) -> bool {
    node.cloud_taints.iter().any(|t| t == key)
}

/// Compute the (additions, removals) needed to make the failure-taint set on
/// the Node match the cloud's reported `state`. Healthy state removes all
/// known failure taints; deletion-bound states do nothing (the controller
/// will delete the Node instead).
pub fn taints_for_state(node: &NodeView, state: InstanceState) -> (Vec<String>, Vec<String>) {
    if state.requires_deletion() {
        return (Vec::new(), Vec::new());
    }
    let want_taint = state.failure_taint().map(String::from);
    let known: [&str; 2] = [SHUTDOWN_TAINT_KEY, UNREACHABLE_TAINT_KEY];
    let mut add = Vec::new();
    let mut remove = Vec::new();
    if let Some(ref t) = want_taint {
        if !has_taint(node, t) {
            add.push(t.clone());
        }
    }
    for k in known {
        if Some(k.to_string()) != want_taint && has_taint(node, k) {
            remove.push(k.to_string());
        }
    }
    (add, remove)
}

// ─── Reconciliation ──────────────────────────────────────────────────────────

/// First-pass reconciler — stays bug-for-bug compatible with the original
/// scaffold so the v1 tests keep passing.
pub fn reconcile(
    node: &NodeView,
    facts: &CloudFacts,
    _tenant: &TenantId,
) -> Result<Reconcile, CloudError> {
    if !is_initialised(node) {
        let writes = label_drift(node, facts);
        return Ok(Reconcile::Annotate(writes));
    }
    if node.initializer_taint_present {
        return Ok(Reconcile::Untaint(1));
    }
    Ok(Reconcile::NoOp)
}

/// Full lifecycle reconciler — initialization, address sync, failure taints,
/// shutdown deletion. Mirrors the order of operations in `syncNode` +
/// `monitorNode` upstream.
pub fn reconcile_full(
    node: &NodeView,
    facts: &CloudFacts,
    state: InstanceState,
    _tenant: &TenantId,
) -> Result<Reconcile, CloudError> {
    if state.requires_deletion() {
        return Ok(Reconcile::Delete(1));
    }
    if !is_initialised(node) {
        return Ok(Reconcile::Annotate(label_drift(node, facts)));
    }
    let (added, removed) = address_diff(&node.addresses, &facts.addresses);
    let address_writes = (added.len() + removed.len()) as u32;
    let (taint_add, taint_remove) = taints_for_state(node, state);
    let taint_writes = (taint_add.len() + taint_remove.len()) as u32;
    if node.initializer_taint_present {
        return Ok(Reconcile::Untaint(1 + taint_writes + address_writes));
    }
    if taint_writes > 0 {
        return Ok(Reconcile::Untaint(taint_writes + address_writes));
    }
    if address_writes > 0 {
        return Ok(Reconcile::Update(address_writes));
    }
    Ok(Reconcile::NoOp)
}

/// Mirrors `tryUpdateNodeName` for the shutdown / terminated path. Returns
/// `Reconcile::Delete(1)` when the cloud reports the node as gone, else a
/// `NoOp` so the watcher can short-circuit.
pub fn handle_shutdown(_node: &NodeView, state: InstanceState) -> Result<Reconcile, CloudError> {
    if state.requires_deletion() {
        Ok(Reconcile::Delete(1))
    } else {
        Ok(Reconcile::NoOp)
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::k8s(
    "staging/src/k8s.io/cloud-provider/controllers/node/node_controller.go",
    "CloudNodeController",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn fresh_node(name: &str) -> NodeView {
        NodeView::fresh(name)
    }

    fn facts() -> CloudFacts {
        CloudFacts {
            provider_id: "hcloud://1234".into(),
            zone: "fsn1-dc14".into(),
            region: "fsn1".into(),
            instance_type: "cpx21".into(),
            addresses: vec![
                NodeAddress::new(NodeAddressType::InternalIP, "10.0.0.7"),
                NodeAddress::new(NodeAddressType::ExternalIP, "203.0.113.5"),
                NodeAddress::new(NodeAddressType::Hostname, "worker-1.example.com"),
            ],
        }
    }

    fn initialised(name: &str, f: &CloudFacts) -> NodeView {
        NodeView {
            name: name.into(),
            provider_id: Some(f.provider_id.clone()),
            zone: Some(f.zone.clone()),
            region: Some(f.region.clone()),
            instance_type: Some(f.instance_type.clone()),
            initializer_taint_present: false,
            cloud_taints: Vec::new(),
            addresses: f.addresses.clone(),
        }
    }

    // ─── Existing v1 tests ───────────────────────────────────────────────────

    #[test]
    fn fresh_node_needs_four_annotations() {
        let (_cite, tenant) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/node/node_controller.go",
            "syncNode",
            "tenant-node-fresh"
        );
        let n = fresh_node("worker-1");
        assert!(!is_initialised(&n));
        assert_eq!(reconcile(&n, &facts(), &tenant).unwrap(), Reconcile::Annotate(4));
    }

    #[test]
    fn partially_annotated_node_writes_only_diff() {
        let (_cite, tenant) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/node/node_controller.go",
            "updateNodeAddress",
            "tenant-node-partial"
        );
        let mut n = fresh_node("worker-2");
        let f = facts();
        n.provider_id = Some(f.provider_id.clone());
        n.region = Some(f.region.clone());
        // Two fields still missing → 2 writes.
        assert_eq!(reconcile(&n, &f, &tenant).unwrap(), Reconcile::Annotate(2));
    }

    #[test]
    fn fully_initialised_node_drops_initializer_taint() {
        let (_cite, tenant) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/node/node_controller.go",
            "ensureNodeExistsByProviderID",
            "tenant-node-untaint"
        );
        let f = facts();
        let n = NodeView {
            initializer_taint_present: true,
            ..initialised("worker-3", &f)
        };
        assert!(is_initialised(&n));
        assert_eq!(reconcile(&n, &f, &tenant).unwrap(), Reconcile::Untaint(1));
    }

    #[test]
    fn fully_initialised_untainted_node_is_a_no_op() {
        let (_cite, tenant) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/node/node_controller.go",
            "syncNode",
            "tenant-node-noop"
        );
        let f = facts();
        let n = initialised("worker-4", &f);
        assert_eq!(reconcile(&n, &f, &tenant).unwrap(), Reconcile::NoOp);
    }

    #[test]
    fn label_constants_match_upstream_keys() {
        let (_cite, tenant) = test_ctx!(
            "staging/src/k8s.io/api/core/v1/well_known_labels.go",
            "LabelTopologyZone",
            "tenant-node-labels"
        );
        let _ = tenant;
        assert_eq!(LABEL_ZONE, "topology.kubernetes.io/zone");
        assert_eq!(LABEL_REGION, "topology.kubernetes.io/region");
        assert_eq!(LABEL_INSTANCE_TYPE, "node.kubernetes.io/instance-type");
        assert_eq!(LABEL_HOSTNAME, "kubernetes.io/hostname");
        assert_eq!(INITIALIZER_TAINT_KEY, "node.cloudprovider.kubernetes.io/uninitialized");
    }

    // ─── Provider-id parsing ─────────────────────────────────────────────────

    #[test]
    fn provider_id_parses_hcloud_scheme_and_id() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/cloud.go",
            "InstanceID",
            "tenant-node-pid-hcloud"
        );
        let (s, id) = parse_provider_id("hcloud://1234").unwrap();
        assert_eq!(s, "hcloud");
        assert_eq!(id, "1234");
    }

    #[test]
    fn provider_id_parses_azure_scheme_and_id() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/cloud.go",
            "InstanceID",
            "tenant-node-pid-azure"
        );
        let (s, id) = parse_provider_id("azure://vmss-app_0").unwrap();
        assert_eq!(s, "azure");
        assert_eq!(id, "vmss-app_0");
    }

    #[test]
    fn provider_id_rejects_missing_scheme_separator() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/cloud.go",
            "InstanceID",
            "tenant-node-pid-bad"
        );
        assert!(parse_provider_id("hcloud-1234").is_none());
        assert!(parse_provider_id("").is_none());
    }

    #[test]
    fn provider_id_rejects_empty_components() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/cloud.go",
            "InstanceID",
            "tenant-node-pid-empty"
        );
        assert!(parse_provider_id("://1234").is_none());
        assert!(parse_provider_id("hcloud://").is_none());
    }

    #[test]
    fn provider_id_scheme_helper_matches_full_parse() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/cloud.go",
            "InstanceID",
            "tenant-node-pid-scheme"
        );
        assert_eq!(provider_id_scheme("azure://x"), Some("azure"));
        assert_eq!(provider_id_scheme("hcloud://7"), Some("hcloud"));
        assert!(provider_id_scheme("not-a-uri").is_none());
    }

    #[test]
    fn provider_id_format_round_trips_through_parse() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/cloud.go",
            "InstanceID",
            "tenant-node-pid-format"
        );
        let s = format_provider_id("hcloud", "1234");
        let (scheme, id) = parse_provider_id(&s).unwrap();
        assert_eq!(scheme, "hcloud");
        assert_eq!(id, "1234");
    }

    // ─── Address types ───────────────────────────────────────────────────────

    #[test]
    fn address_type_keys_match_upstream_strings() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/api/core/v1/types.go",
            "NodeAddressType",
            "tenant-node-addr-keys"
        );
        assert_eq!(NodeAddressType::InternalIP.key(), "InternalIP");
        assert_eq!(NodeAddressType::ExternalIP.key(), "ExternalIP");
        assert_eq!(NodeAddressType::InternalDNS.key(), "InternalDNS");
        assert_eq!(NodeAddressType::ExternalDNS.key(), "ExternalDNS");
        assert_eq!(NodeAddressType::Hostname.key(), "Hostname");
    }

    #[test]
    fn address_type_precedence_orders_internal_first() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/api/core/v1/types.go",
            "NodeAddressType",
            "tenant-node-addr-prec"
        );
        assert!(NodeAddressType::InternalIP.precedence() < NodeAddressType::ExternalIP.precedence());
        assert!(NodeAddressType::ExternalIP.precedence() < NodeAddressType::Hostname.precedence());
    }

    #[test]
    fn canonicalize_addresses_dedupes_and_sorts_by_precedence() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/node/helpers/address.go",
            "mergeNodeAddresses",
            "tenant-node-addr-canon"
        );
        let raw = vec![
            NodeAddress::new(NodeAddressType::Hostname, "h1"),
            NodeAddress::new(NodeAddressType::ExternalIP, "1.1.1.1"),
            NodeAddress::new(NodeAddressType::InternalIP, "10.0.0.1"),
            NodeAddress::new(NodeAddressType::InternalIP, "10.0.0.1"), // dup
        ];
        let out = canonicalize_addresses(&raw);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].kind, NodeAddressType::InternalIP);
        assert_eq!(out[1].kind, NodeAddressType::ExternalIP);
        assert_eq!(out[2].kind, NodeAddressType::Hostname);
    }

    #[test]
    fn preferred_address_returns_internal_ip_when_present() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/node/helpers/address.go",
            "preferredNodeAddress",
            "tenant-node-addr-pref"
        );
        let addrs = vec![
            NodeAddress::new(NodeAddressType::ExternalIP, "1.1.1.1"),
            NodeAddress::new(NodeAddressType::InternalIP, "10.0.0.1"),
        ];
        assert_eq!(
            preferred_address(&addrs).map(|a| (a.kind, a.address.as_str())),
            Some((NodeAddressType::InternalIP, "10.0.0.1"))
        );
    }

    #[test]
    fn preferred_address_falls_back_to_hostname_only_node() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/node/helpers/address.go",
            "preferredNodeAddress",
            "tenant-node-addr-host"
        );
        let addrs = vec![NodeAddress::new(NodeAddressType::Hostname, "h.example.com")];
        assert_eq!(preferred_address(&addrs).unwrap().kind, NodeAddressType::Hostname);
    }

    #[test]
    fn preferred_address_returns_none_for_empty_set() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/node/helpers/address.go",
            "preferredNodeAddress",
            "tenant-node-addr-empty"
        );
        assert!(preferred_address(&[]).is_none());
    }

    #[test]
    fn address_diff_detects_added_addresses() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/node/node_controller.go",
            "syncNodeAddress",
            "tenant-node-addr-add"
        );
        let have = vec![NodeAddress::new(NodeAddressType::InternalIP, "10.0.0.1")];
        let want = vec![
            NodeAddress::new(NodeAddressType::InternalIP, "10.0.0.1"),
            NodeAddress::new(NodeAddressType::ExternalIP, "1.1.1.1"),
        ];
        let (added, removed) = address_diff(&have, &want);
        assert_eq!(added.len(), 1);
        assert_eq!(added[0].kind, NodeAddressType::ExternalIP);
        assert!(removed.is_empty());
    }

    #[test]
    fn address_diff_detects_removed_addresses() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/node/node_controller.go",
            "syncNodeAddress",
            "tenant-node-addr-rm"
        );
        let have = vec![
            NodeAddress::new(NodeAddressType::InternalIP, "10.0.0.1"),
            NodeAddress::new(NodeAddressType::ExternalIP, "1.1.1.1"),
        ];
        let want = vec![NodeAddress::new(NodeAddressType::InternalIP, "10.0.0.1")];
        let (added, removed) = address_diff(&have, &want);
        assert!(added.is_empty());
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].kind, NodeAddressType::ExternalIP);
    }

    #[test]
    fn address_diff_returns_empty_for_identical_sets() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/node/node_controller.go",
            "syncNodeAddress",
            "tenant-node-addr-same"
        );
        let v = vec![NodeAddress::new(NodeAddressType::InternalIP, "10.0.0.1")];
        let (a, r) = address_diff(&v, &v);
        assert!(a.is_empty() && r.is_empty());
    }

    #[test]
    fn address_diff_treats_kind_change_as_add_plus_remove() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/node/node_controller.go",
            "syncNodeAddress",
            "tenant-node-addr-flip"
        );
        let have = vec![NodeAddress::new(NodeAddressType::InternalIP, "10.0.0.1")];
        let want = vec![NodeAddress::new(NodeAddressType::ExternalIP, "10.0.0.1")];
        let (added, removed) = address_diff(&have, &want);
        assert_eq!(added.len(), 1);
        assert_eq!(removed.len(), 1);
    }

    // ─── Label drift ─────────────────────────────────────────────────────────

    #[test]
    fn label_drift_is_zero_when_node_matches_facts() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/node/node_controller.go",
            "reconcileLabels",
            "tenant-node-drift-zero"
        );
        let f = facts();
        let n = initialised("w", &f);
        assert_eq!(label_drift(&n, &f), 0);
    }

    #[test]
    fn label_drift_counts_changed_zone() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/node/node_controller.go",
            "reconcileLabels",
            "tenant-node-drift-zone"
        );
        let f = facts();
        let mut n = initialised("w", &f);
        n.zone = Some("nbg1-dc3".into());
        assert_eq!(label_drift(&n, &f), 1);
    }

    #[test]
    fn label_drift_counts_each_independent_field() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/node/node_controller.go",
            "reconcileLabels",
            "tenant-node-drift-many"
        );
        let f = facts();
        let mut n = initialised("w", &f);
        n.zone = Some("zzz".into());
        n.region = Some("rrr".into());
        n.instance_type = Some("ttt".into());
        assert_eq!(label_drift(&n, &f), 3);
    }

    // ─── Instance state ──────────────────────────────────────────────────────

    #[test]
    fn instance_state_running_is_healthy_and_taint_free() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/cloud.go",
            "InstanceMetadataStatus",
            "tenant-node-state-run"
        );
        assert!(InstanceState::Running.is_healthy());
        assert!(InstanceState::Running.failure_taint().is_none());
    }

    #[test]
    fn instance_state_shutdown_emits_shutdown_taint() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/node/node_lifecycle_controller.go",
            "shutdownTaint",
            "tenant-node-state-down"
        );
        assert_eq!(
            InstanceState::Shutdown.failure_taint(),
            Some(SHUTDOWN_TAINT_KEY)
        );
        assert!(!InstanceState::Shutdown.is_healthy());
    }

    #[test]
    fn instance_state_unreachable_emits_unreachable_taint() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/api/core/v1/well_known_taints.go",
            "TaintNodeUnreachable",
            "tenant-node-state-unreach"
        );
        assert_eq!(
            InstanceState::Unreachable.failure_taint(),
            Some(UNREACHABLE_TAINT_KEY)
        );
    }

    #[test]
    fn instance_state_terminated_requires_deletion() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/node/node_controller.go",
            "ensureNodeExistsByProviderID",
            "tenant-node-state-term"
        );
        assert!(InstanceState::Terminated.requires_deletion());
        assert!(InstanceState::NotFound.requires_deletion());
        assert!(!InstanceState::Running.requires_deletion());
        assert!(!InstanceState::Shutdown.requires_deletion());
    }

    #[test]
    fn taint_key_constants_match_upstream() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/api/core/v1/well_known_taints.go",
            "TaintNodeShutdown",
            "tenant-node-taint-keys"
        );
        assert_eq!(SHUTDOWN_TAINT_KEY, "node.cloudprovider.kubernetes.io/shutdown");
        assert_eq!(UNREACHABLE_TAINT_KEY, "node.kubernetes.io/unreachable");
        assert_eq!(NOT_READY_TAINT_KEY, "node.kubernetes.io/not-ready");
        assert_eq!(OUT_OF_SERVICE_TAINT_KEY, "node.kubernetes.io/out-of-service");
    }

    // ─── Taint reconciliation ────────────────────────────────────────────────

    #[test]
    fn has_taint_reads_cloud_taint_list() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/node/node_lifecycle_controller.go",
            "hasTaint",
            "tenant-node-has-taint"
        );
        let mut n = NodeView::fresh("n");
        n.cloud_taints.push(SHUTDOWN_TAINT_KEY.into());
        assert!(has_taint(&n, SHUTDOWN_TAINT_KEY));
        assert!(!has_taint(&n, UNREACHABLE_TAINT_KEY));
    }

    #[test]
    fn taints_for_state_running_clears_all_failure_taints() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/node/node_lifecycle_controller.go",
            "monitorNodeHealth",
            "tenant-node-taint-clear"
        );
        let f = facts();
        let mut n = initialised("w", &f);
        n.cloud_taints = vec![SHUTDOWN_TAINT_KEY.into(), UNREACHABLE_TAINT_KEY.into()];
        let (add, remove) = taints_for_state(&n, InstanceState::Running);
        assert!(add.is_empty());
        assert_eq!(remove.len(), 2);
    }

    #[test]
    fn taints_for_state_shutdown_swaps_unreachable_for_shutdown() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/node/node_lifecycle_controller.go",
            "applyShutdownTaint",
            "tenant-node-taint-swap"
        );
        let f = facts();
        let mut n = initialised("w", &f);
        n.cloud_taints = vec![UNREACHABLE_TAINT_KEY.into()];
        let (add, remove) = taints_for_state(&n, InstanceState::Shutdown);
        assert_eq!(add, vec![SHUTDOWN_TAINT_KEY.to_string()]);
        assert_eq!(remove, vec![UNREACHABLE_TAINT_KEY.to_string()]);
    }

    #[test]
    fn taints_for_state_shutdown_is_idempotent_when_taint_present() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/node/node_lifecycle_controller.go",
            "applyShutdownTaint",
            "tenant-node-taint-idem"
        );
        let f = facts();
        let mut n = initialised("w", &f);
        n.cloud_taints = vec![SHUTDOWN_TAINT_KEY.into()];
        let (add, remove) = taints_for_state(&n, InstanceState::Shutdown);
        assert!(add.is_empty());
        assert!(remove.is_empty());
    }

    #[test]
    fn taints_for_state_terminated_reports_no_writes() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/node/node_controller.go",
            "ensureNodeExistsByProviderID",
            "tenant-node-taint-term"
        );
        let f = facts();
        let mut n = initialised("w", &f);
        n.cloud_taints = vec![UNREACHABLE_TAINT_KEY.into()];
        let (add, remove) = taints_for_state(&n, InstanceState::Terminated);
        assert!(add.is_empty() && remove.is_empty());
    }

    // ─── reconcile_full ──────────────────────────────────────────────────────

    #[test]
    fn reconcile_full_uninitialised_node_emits_annotate() {
        let (_cite, tenant) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/node/node_controller.go",
            "syncNode",
            "tenant-node-full-init"
        );
        let f = facts();
        let n = fresh_node("w");
        let r = reconcile_full(&n, &f, InstanceState::Running, &tenant).unwrap();
        assert_eq!(r, Reconcile::Annotate(4));
    }

    #[test]
    fn reconcile_full_terminated_state_emits_delete_immediately() {
        let (_cite, tenant) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/node/node_controller.go",
            "ensureNodeExistsByProviderID",
            "tenant-node-full-term"
        );
        let f = facts();
        let n = initialised("w", &f);
        let r = reconcile_full(&n, &f, InstanceState::Terminated, &tenant).unwrap();
        assert_eq!(r, Reconcile::Delete(1));
    }

    #[test]
    fn reconcile_full_not_found_state_emits_delete() {
        let (_cite, tenant) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/node/node_controller.go",
            "ensureNodeExistsByProviderID",
            "tenant-node-full-nf"
        );
        let f = facts();
        let n = initialised("w", &f);
        let r = reconcile_full(&n, &f, InstanceState::NotFound, &tenant).unwrap();
        assert_eq!(r, Reconcile::Delete(1));
    }

    #[test]
    fn reconcile_full_shutdown_emits_untaint_with_taint_writes() {
        let (_cite, tenant) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/node/node_lifecycle_controller.go",
            "applyShutdownTaint",
            "tenant-node-full-down"
        );
        let f = facts();
        let n = initialised("w", &f);
        let r = reconcile_full(&n, &f, InstanceState::Shutdown, &tenant).unwrap();
        // 1 taint addition, no address writes, no initializer.
        assert_eq!(r, Reconcile::Untaint(1));
    }

    #[test]
    fn reconcile_full_running_with_address_drift_emits_update() {
        let (_cite, tenant) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/node/node_controller.go",
            "syncNodeAddress",
            "tenant-node-full-addrs"
        );
        let f = facts();
        let mut n = initialised("w", &f);
        n.addresses.pop(); // drop the hostname → 1 add needed
        let r = reconcile_full(&n, &f, InstanceState::Running, &tenant).unwrap();
        assert_eq!(r, Reconcile::Update(1));
    }

    #[test]
    fn reconcile_full_running_with_initializer_taint_emits_untaint_with_address_count() {
        let (_cite, tenant) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/node/node_controller.go",
            "syncNode",
            "tenant-node-full-init-and-addrs"
        );
        let f = facts();
        let mut n = initialised("w", &f);
        n.initializer_taint_present = true;
        n.addresses.clear(); // 3 missing addresses
        let r = reconcile_full(&n, &f, InstanceState::Running, &tenant).unwrap();
        // 1 (untaint) + 0 (taint writes) + 3 (address adds)
        assert_eq!(r, Reconcile::Untaint(4));
    }

    #[test]
    fn reconcile_full_steady_state_is_a_no_op() {
        let (_cite, tenant) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/node/node_controller.go",
            "syncNode",
            "tenant-node-full-noop"
        );
        let f = facts();
        let n = initialised("w", &f);
        let r = reconcile_full(&n, &f, InstanceState::Running, &tenant).unwrap();
        assert_eq!(r, Reconcile::NoOp);
    }

    // ─── handle_shutdown ─────────────────────────────────────────────────────

    #[test]
    fn handle_shutdown_emits_delete_for_terminated() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/node/node_controller.go",
            "InstanceShutdownByProviderID",
            "tenant-node-hs-term"
        );
        let n = NodeView::fresh("n");
        assert_eq!(handle_shutdown(&n, InstanceState::Terminated).unwrap(), Reconcile::Delete(1));
    }

    #[test]
    fn handle_shutdown_emits_delete_for_not_found() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/node/node_controller.go",
            "InstanceShutdownByProviderID",
            "tenant-node-hs-nf"
        );
        let n = NodeView::fresh("n");
        assert_eq!(handle_shutdown(&n, InstanceState::NotFound).unwrap(), Reconcile::Delete(1));
    }

    #[test]
    fn handle_shutdown_returns_noop_for_running_or_shutdown() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/node/node_controller.go",
            "InstanceShutdownByProviderID",
            "tenant-node-hs-noop"
        );
        let n = NodeView::fresh("n");
        assert_eq!(handle_shutdown(&n, InstanceState::Running).unwrap(), Reconcile::NoOp);
        assert_eq!(handle_shutdown(&n, InstanceState::Shutdown).unwrap(), Reconcile::NoOp);
        assert_eq!(handle_shutdown(&n, InstanceState::Unreachable).unwrap(), Reconcile::NoOp);
    }

    // ─── NodeView::fresh ─────────────────────────────────────────────────────

    #[test]
    fn fresh_constructor_marks_node_uninitialised_with_taint() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/node/node_controller.go",
            "newCloudNodeController",
            "tenant-node-fresh-ctor"
        );
        let n = NodeView::fresh("worker-x");
        assert!(!is_initialised(&n));
        assert!(n.initializer_taint_present);
        assert!(n.addresses.is_empty());
        assert!(n.cloud_taints.is_empty());
    }

    #[test]
    fn cloud_facts_minimal_constructor_is_empty_addresses() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/cloud.go",
            "InstanceMetadata",
            "tenant-node-facts-min"
        );
        let f = CloudFacts::minimal("hcloud://1", "fsn1-dc14", "fsn1", "cpx21");
        assert_eq!(f.provider_id, "hcloud://1");
        assert!(f.addresses.is_empty());
    }
}

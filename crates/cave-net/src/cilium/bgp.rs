//! BGP control plane — `cilium-operator` BGP integration.
//!
//! Mirrors `pkg/bgpv1/manager/manager.go` (the per-instance BGP daemon
//! state), `pkg/bgpv1/agent/controller.go` (peer lifecycle FSM), the
//! `CiliumBGPPeerConfig` / `CiliumBGPAdvertisement` CRDs from
//! `pkg/k8s/apis/cilium.io/v2alpha1/bgp_types.go`, and the path-
//! selection rules from `pkg/bgpv1/manager/reconcile.go`.

use crate::cilium::types::{Cite, TenantId};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::net::IpAddr;

// ── Peer FSM ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PeerState {
    /// RFC 4271: initial state.
    Idle,
    /// FSM in TCP-connect attempt.
    Connect,
    /// Connect failed; trying again from listening side.
    Active,
    /// Sent OPEN; waiting for peer's OPEN.
    OpenSent,
    /// Got peer OPEN; waiting for KEEPALIVE.
    OpenConfirm,
    /// Session up; UPDATE messages may flow.
    Established,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PeerEvent {
    Start,
    TcpConnected,
    TcpFailed,
    OpenReceived,
    KeepaliveReceived,
    NotificationReceived,
    HoldTimerExpired,
    Stop,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum BgpError {
    #[error("invalid peer FSM transition {from:?} via {event:?}")]
    BadTransition { from: PeerState, event: PeerEvent },
    #[error("peer `{0}` not found")]
    PeerNotFound(String),
    #[error("route `{0}` not found")]
    RouteNotFound(String),
    #[error("tenant {tenant} cannot mutate BGP instance owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

// ── BGP attributes / route ──────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PathAttributes {
    pub origin: PathOrigin,
    pub as_path: Vec<u32>,
    pub next_hop: IpAddr,
    pub local_pref: u32,
    pub med: u32,
    pub communities: Vec<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PathOrigin {
    Igp,
    Egp,
    Incomplete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AdvertisementKind {
    PodCidr,
    LoadBalancerIp,
    ExternalIp,
    ServiceCidr,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Advertisement {
    pub name: String,
    pub kind: AdvertisementKind,
    pub prefix: String,
    pub attributes: PathAttributes,
}

/// `NO_EXPORT` is a well-known community (0xFFFFFF01) that prevents
/// the route from being announced past the local AS. Mirrors
/// `pkg/bgpv1/manager/reconcile.go::handleCommunities`.
pub const COMMUNITY_NO_EXPORT: u32 = 0xFFFFFF01;
pub const COMMUNITY_NO_ADVERTISE: u32 = 0xFFFFFF02;

impl Advertisement {
    pub fn has_no_export(&self) -> bool {
        self.attributes.communities.contains(&COMMUNITY_NO_EXPORT)
    }
    pub fn has_no_advertise(&self) -> bool {
        self.attributes.communities.contains(&COMMUNITY_NO_ADVERTISE)
    }
}

// ── Peer config ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerConfig {
    pub name: String,
    pub peer_address: IpAddr,
    pub peer_asn: u32,
    pub local_asn: u32,
    pub hold_time_seconds: u16,
    pub keepalive_seconds: u16,
    pub auth_md5_secret: Option<String>,
    pub families: Vec<AddressFamily>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AddressFamily {
    Ipv4Unicast,
    Ipv6Unicast,
}

impl PeerConfig {
    /// Default timers from RFC 4271: hold = 90s, keepalive = hold/3.
    pub fn defaults(name: impl Into<String>, peer_address: IpAddr, peer_asn: u32, local_asn: u32) -> Self {
        Self {
            name: name.into(), peer_address, peer_asn, local_asn,
            hold_time_seconds: 90, keepalive_seconds: 30,
            auth_md5_secret: None,
            families: vec![AddressFamily::Ipv4Unicast],
        }
    }
}

// ── BGP instance ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerStatus {
    pub config: PeerConfig,
    pub state: PeerState,
    pub last_event: Option<PeerEvent>,
    pub uptime_seconds: u64,
}

#[derive(Debug)]
pub struct BgpInstance {
    pub tenant: TenantId,
    pub local_asn: u32,
    pub router_id: IpAddr,
    peers: HashMap<String, PeerStatus>,
    advertisements: BTreeMap<String, Advertisement>,
    /// Per-prefix incoming paths (peer name → path attributes).
    incoming_paths: BTreeMap<String, Vec<(String, PathAttributes)>>,
}

impl BgpInstance {
    pub fn new(tenant: TenantId, local_asn: u32, router_id: IpAddr) -> Self {
        Self {
            tenant, local_asn, router_id,
            peers: HashMap::new(),
            advertisements: BTreeMap::new(),
            incoming_paths: BTreeMap::new(),
        }
    }

    pub fn upsert_peer(&mut self, config: PeerConfig) {
        self.peers.insert(config.name.clone(), PeerStatus {
            state: PeerState::Idle,
            last_event: None,
            uptime_seconds: 0,
            config,
        });
    }

    pub fn remove_peer(&mut self, name: &str) -> Result<(), BgpError> {
        self.peers.remove(name).ok_or_else(|| BgpError::PeerNotFound(name.to_string()))?;
        Ok(())
    }

    pub fn peer_count(&self) -> usize {
        self.peers.len()
    }

    pub fn peer_status(&self, name: &str) -> Option<&PeerStatus> {
        self.peers.get(name)
    }

    /// Drive the peer FSM with an event. Mirrors RFC 4271 §8.
    pub fn handle_event(&mut self, peer: &str, event: PeerEvent) -> Result<PeerState, BgpError> {
        let p = self.peers.get_mut(peer).ok_or_else(|| BgpError::PeerNotFound(peer.to_string()))?;
        let next = match (p.state, event) {
            // Start path.
            (PeerState::Idle, PeerEvent::Start) => PeerState::Connect,
            (PeerState::Connect, PeerEvent::TcpConnected) => PeerState::OpenSent,
            (PeerState::Connect, PeerEvent::TcpFailed) => PeerState::Active,
            (PeerState::Active, PeerEvent::TcpConnected) => PeerState::OpenSent,
            (PeerState::OpenSent, PeerEvent::OpenReceived) => PeerState::OpenConfirm,
            (PeerState::OpenConfirm, PeerEvent::KeepaliveReceived) => PeerState::Established,
            // Refresh keepalive while established → no-op.
            (PeerState::Established, PeerEvent::KeepaliveReceived) => PeerState::Established,
            // Failure paths back to Idle.
            (_, PeerEvent::Stop) => PeerState::Idle,
            (_, PeerEvent::NotificationReceived) => PeerState::Idle,
            (PeerState::Established, PeerEvent::HoldTimerExpired) => PeerState::Idle,
            (PeerState::OpenSent | PeerState::OpenConfirm, PeerEvent::HoldTimerExpired) => PeerState::Idle,
            (from, ev) => return Err(BgpError::BadTransition { from, event: ev }),
        };
        p.state = next;
        p.last_event = Some(event);
        if matches!(next, PeerState::Established) {
            p.uptime_seconds = 0;
        }
        Ok(next)
    }

    // ── Advertisement management ─────────────────────────────────────────────

    pub fn advertise(&mut self, ad: Advertisement) {
        self.advertisements.insert(ad.name.clone(), ad);
    }

    pub fn withdraw(&mut self, name: &str) -> Result<(), BgpError> {
        self.advertisements.remove(name).ok_or_else(|| BgpError::RouteNotFound(name.to_string()))?;
        Ok(())
    }

    pub fn advertisements(&self) -> Vec<&Advertisement> {
        self.advertisements.values().collect()
    }

    pub fn advertisement(&self, name: &str) -> Option<&Advertisement> {
        self.advertisements.get(name)
    }

    /// Effective advertisements for a given peer — filters out routes
    /// flagged NO_EXPORT when the peer is in a different ASN, and routes
    /// flagged NO_ADVERTISE always (mirrors RFC 1997).
    pub fn effective_advertisements_for(&self, peer: &str) -> Result<Vec<&Advertisement>, BgpError> {
        let p = self.peers.get(peer).ok_or_else(|| BgpError::PeerNotFound(peer.to_string()))?;
        let cross_as = p.config.peer_asn != self.local_asn;
        Ok(self.advertisements.values()
            .filter(|a| !a.has_no_advertise())
            .filter(|a| !(cross_as && a.has_no_export()))
            .collect())
    }

    // ── Path selection ───────────────────────────────────────────────────────

    pub fn record_incoming_path(&mut self, prefix: impl Into<String>, peer: impl Into<String>, attrs: PathAttributes) {
        let prefix = prefix.into();
        self.incoming_paths.entry(prefix).or_default().push((peer.into(), attrs));
    }

    /// Pick the best path for `prefix` using RFC 4271 tie-breaking:
    /// 1. Highest LOCAL_PREF.
    /// 2. Shortest AS_PATH.
    /// 3. Lowest origin (IGP < EGP < Incomplete).
    /// 4. Lowest MED.
    /// 5. Lexicographic peer name (deterministic tie-break).
    pub fn best_path(&self, prefix: &str) -> Option<(&str, &PathAttributes)> {
        let candidates = self.incoming_paths.get(prefix)?;
        if candidates.is_empty() {
            return None;
        }
        let mut best: Option<&(String, PathAttributes)> = None;
        for c in candidates {
            best = Some(match best {
                None => c,
                Some(b) => {
                    let cur = b;
                    let cmp = path_compare(&c.1, &cur.1);
                    if cmp == std::cmp::Ordering::Greater {
                        c
                    } else if cmp == std::cmp::Ordering::Equal && c.0 < cur.0 {
                        c
                    } else {
                        cur
                    }
                }
            });
        }
        best.map(|(p, a)| (p.as_str(), a))
    }

    pub fn paths_for(&self, prefix: &str) -> Vec<(&str, &PathAttributes)> {
        self.incoming_paths.get(prefix)
            .map(|v| v.iter().map(|(p, a)| (p.as_str(), a)).collect())
            .unwrap_or_default()
    }
}

/// Compare two BGP paths — `Greater` means `a` is *better* than `b`.
fn path_compare(a: &PathAttributes, b: &PathAttributes) -> std::cmp::Ordering {
    use std::cmp::Ordering::*;
    if a.local_pref != b.local_pref {
        return a.local_pref.cmp(&b.local_pref);
    }
    if a.as_path.len() != b.as_path.len() {
        return b.as_path.len().cmp(&a.as_path.len()); // shorter is better → reversed
    }
    let origin_rank = |o: PathOrigin| match o { PathOrigin::Igp => 0, PathOrigin::Egp => 1, PathOrigin::Incomplete => 2 };
    let oa = origin_rank(a.origin);
    let ob = origin_rank(b.origin);
    if oa != ob {
        return ob.cmp(&oa); // lower origin rank is better → reversed
    }
    if a.med != b.med {
        return b.med.cmp(&a.med); // lower MED is better → reversed
    }
    Equal
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/bgpv1/manager/manager.go", "BGPInstance");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;
    use std::net::Ipv4Addr;

    fn ip(a: u8, b: u8, c: u8, d: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(a, b, c, d))
    }

    fn instance(tenant: TenantId) -> BgpInstance {
        BgpInstance::new(tenant, 65000, ip(10, 0, 0, 1))
    }

    fn peer(asn: u32) -> PeerConfig {
        PeerConfig::defaults("peer-a", ip(10, 0, 0, 2), asn, 65000)
    }

    fn attrs() -> PathAttributes {
        PathAttributes {
            origin: PathOrigin::Igp,
            as_path: vec![65000],
            next_hop: ip(10, 0, 0, 1),
            local_pref: 100,
            med: 0,
            communities: vec![],
        }
    }

    // ── Peer FSM ─────────────────────────────────────────────────────────────

    #[test]
    fn bgp_peer_initial_state_idle() {
        let (_c, tenant) = cilium_test_ctx!("pkg/bgpv1/agent/controller.go", "Peer.InitialState", "tenant-bgp-init");
        let mut i = instance(tenant);
        i.upsert_peer(peer(64999));
        assert_eq!(i.peer_status("peer-a").unwrap().state, PeerState::Idle);
    }

    #[test]
    fn bgp_peer_start_advances_to_connect() {
        let (_c, tenant) = cilium_test_ctx!("pkg/bgpv1/agent/controller.go", "Peer.Start", "tenant-bgp-start");
        let mut i = instance(tenant);
        i.upsert_peer(peer(64999));
        let s = i.handle_event("peer-a", PeerEvent::Start).unwrap();
        assert_eq!(s, PeerState::Connect);
    }

    #[test]
    fn bgp_peer_tcp_connected_advances_to_open_sent() {
        let (_c, tenant) = cilium_test_ctx!("pkg/bgpv1/agent/controller.go", "Peer.OpenSent", "tenant-bgp-os");
        let mut i = instance(tenant);
        i.upsert_peer(peer(64999));
        i.handle_event("peer-a", PeerEvent::Start).unwrap();
        let s = i.handle_event("peer-a", PeerEvent::TcpConnected).unwrap();
        assert_eq!(s, PeerState::OpenSent);
    }

    #[test]
    fn bgp_peer_tcp_failed_advances_to_active() {
        let (_c, tenant) = cilium_test_ctx!("pkg/bgpv1/agent/controller.go", "Peer.Active", "tenant-bgp-act");
        let mut i = instance(tenant);
        i.upsert_peer(peer(64999));
        i.handle_event("peer-a", PeerEvent::Start).unwrap();
        let s = i.handle_event("peer-a", PeerEvent::TcpFailed).unwrap();
        assert_eq!(s, PeerState::Active);
    }

    #[test]
    fn bgp_peer_full_handshake_to_established() {
        let (_c, tenant) = cilium_test_ctx!("pkg/bgpv1/agent/controller.go", "Peer.Established", "tenant-bgp-est");
        let mut i = instance(tenant);
        i.upsert_peer(peer(64999));
        i.handle_event("peer-a", PeerEvent::Start).unwrap();
        i.handle_event("peer-a", PeerEvent::TcpConnected).unwrap();
        i.handle_event("peer-a", PeerEvent::OpenReceived).unwrap();
        let s = i.handle_event("peer-a", PeerEvent::KeepaliveReceived).unwrap();
        assert_eq!(s, PeerState::Established);
    }

    #[test]
    fn bgp_peer_keepalive_in_established_stays_established() {
        let (_c, tenant) = cilium_test_ctx!("pkg/bgpv1/agent/controller.go", "Peer.KeepaliveRefresh", "tenant-bgp-kar");
        let mut i = instance(tenant);
        i.upsert_peer(peer(64999));
        i.handle_event("peer-a", PeerEvent::Start).unwrap();
        i.handle_event("peer-a", PeerEvent::TcpConnected).unwrap();
        i.handle_event("peer-a", PeerEvent::OpenReceived).unwrap();
        i.handle_event("peer-a", PeerEvent::KeepaliveReceived).unwrap();
        let s = i.handle_event("peer-a", PeerEvent::KeepaliveReceived).unwrap();
        assert_eq!(s, PeerState::Established);
    }

    #[test]
    fn bgp_peer_notification_drops_back_to_idle() {
        let (_c, tenant) = cilium_test_ctx!("pkg/bgpv1/agent/controller.go", "Peer.Notification", "tenant-bgp-nt");
        let mut i = instance(tenant);
        i.upsert_peer(peer(64999));
        i.handle_event("peer-a", PeerEvent::Start).unwrap();
        i.handle_event("peer-a", PeerEvent::TcpConnected).unwrap();
        i.handle_event("peer-a", PeerEvent::OpenReceived).unwrap();
        i.handle_event("peer-a", PeerEvent::KeepaliveReceived).unwrap();
        let s = i.handle_event("peer-a", PeerEvent::NotificationReceived).unwrap();
        assert_eq!(s, PeerState::Idle);
    }

    #[test]
    fn bgp_peer_hold_timer_expired_in_established_drops_to_idle() {
        let (_c, tenant) = cilium_test_ctx!("pkg/bgpv1/agent/controller.go", "Peer.HoldTimerExpired", "tenant-bgp-hte");
        let mut i = instance(tenant);
        i.upsert_peer(peer(64999));
        i.handle_event("peer-a", PeerEvent::Start).unwrap();
        i.handle_event("peer-a", PeerEvent::TcpConnected).unwrap();
        i.handle_event("peer-a", PeerEvent::OpenReceived).unwrap();
        i.handle_event("peer-a", PeerEvent::KeepaliveReceived).unwrap();
        let s = i.handle_event("peer-a", PeerEvent::HoldTimerExpired).unwrap();
        assert_eq!(s, PeerState::Idle);
    }

    #[test]
    fn bgp_peer_invalid_transition_rejected() {
        let (_c, tenant) = cilium_test_ctx!("pkg/bgpv1/agent/controller.go", "Peer.BadTransition", "tenant-bgp-bad");
        let mut i = instance(tenant);
        i.upsert_peer(peer(64999));
        let err = i.handle_event("peer-a", PeerEvent::OpenReceived).unwrap_err();
        assert!(matches!(err, BgpError::BadTransition { .. }));
    }

    #[test]
    fn bgp_peer_event_on_unknown_peer_returns_not_found() {
        let (_c, tenant) = cilium_test_ctx!("pkg/bgpv1/agent/controller.go", "Peer.NotFound", "tenant-bgp-nf");
        let mut i = instance(tenant);
        let err = i.handle_event("ghost", PeerEvent::Start).unwrap_err();
        assert!(matches!(err, BgpError::PeerNotFound(_)));
    }

    #[test]
    fn bgp_peer_stop_returns_to_idle_from_any_state() {
        let (_c, tenant) = cilium_test_ctx!("pkg/bgpv1/agent/controller.go", "Peer.Stop", "tenant-bgp-stop");
        let mut i = instance(tenant);
        i.upsert_peer(peer(64999));
        i.handle_event("peer-a", PeerEvent::Start).unwrap();
        i.handle_event("peer-a", PeerEvent::TcpConnected).unwrap();
        let s = i.handle_event("peer-a", PeerEvent::Stop).unwrap();
        assert_eq!(s, PeerState::Idle);
    }

    // ── Peer config ──────────────────────────────────────────────────────────

    #[test]
    fn bgp_peer_config_defaults_use_rfc_timers() {
        let (_c, _t) = cilium_test_ctx!("pkg/bgpv1/manager/reconcile.go", "PeerConfig.Defaults", "tenant-bgp-cfg");
        let p = PeerConfig::defaults("peer-a", ip(10, 0, 0, 2), 64999, 65000);
        assert_eq!(p.hold_time_seconds, 90);
        assert_eq!(p.keepalive_seconds, 30);
    }

    #[test]
    fn bgp_peer_config_dual_stack_families() {
        let (_c, _t) = cilium_test_ctx!("pkg/k8s/apis/cilium.io/v2alpha1/bgp_types.go", "PeerConfig.Families", "tenant-bgp-fam");
        let mut p = peer(64999);
        p.families = vec![AddressFamily::Ipv4Unicast, AddressFamily::Ipv6Unicast];
        assert_eq!(p.families.len(), 2);
    }

    #[test]
    fn bgp_peer_config_md5_authentication() {
        let (_c, _t) = cilium_test_ctx!("pkg/bgpv1/manager/reconcile.go", "PeerConfig.AuthMD5", "tenant-bgp-md5");
        let mut p = peer(64999);
        p.auth_md5_secret = Some("shared-secret".into());
        assert!(p.auth_md5_secret.is_some());
    }

    // ── Advertisement management ─────────────────────────────────────────────

    #[test]
    fn bgp_advertise_pod_cidr() {
        let (_c, tenant) = cilium_test_ctx!("pkg/bgpv1/manager/reconcile.go", "Advertise.PodCIDR", "tenant-bgp-pod");
        let mut i = instance(tenant);
        i.advertise(Advertisement {
            name: "pods".into(), kind: AdvertisementKind::PodCidr,
            prefix: "10.244.1.0/24".into(), attributes: attrs(),
        });
        assert_eq!(i.advertisements().len(), 1);
    }

    #[test]
    fn bgp_advertise_loadbalancer_ip() {
        let (_c, tenant) = cilium_test_ctx!("pkg/bgpv1/manager/reconcile.go", "Advertise.LBIP", "tenant-bgp-lb");
        let mut i = instance(tenant);
        i.advertise(Advertisement {
            name: "lb".into(), kind: AdvertisementKind::LoadBalancerIp,
            prefix: "203.0.113.10/32".into(), attributes: attrs(),
        });
        assert_eq!(i.advertisement("lb").unwrap().kind, AdvertisementKind::LoadBalancerIp);
    }

    #[test]
    fn bgp_withdraw_advertisement() {
        let (_c, tenant) = cilium_test_ctx!("pkg/bgpv1/manager/reconcile.go", "Advertise.Withdraw", "tenant-bgp-wd");
        let mut i = instance(tenant);
        i.advertise(Advertisement {
            name: "pods".into(), kind: AdvertisementKind::PodCidr,
            prefix: "10.244.1.0/24".into(), attributes: attrs(),
        });
        i.withdraw("pods").unwrap();
        assert!(i.advertisement("pods").is_none());
    }

    #[test]
    fn bgp_withdraw_unknown_returns_not_found() {
        let (_c, tenant) = cilium_test_ctx!("pkg/bgpv1/manager/reconcile.go", "Advertise.Withdraw.NotFound", "tenant-bgp-wdnf");
        let mut i = instance(tenant);
        let err = i.withdraw("ghost").unwrap_err();
        assert!(matches!(err, BgpError::RouteNotFound(_)));
    }

    #[test]
    fn bgp_advertise_idempotent_replaces_in_place() {
        let (_c, tenant) = cilium_test_ctx!("pkg/bgpv1/manager/reconcile.go", "Advertise.Idempotent", "tenant-bgp-idem");
        let mut i = instance(tenant);
        i.advertise(Advertisement {
            name: "pods".into(), kind: AdvertisementKind::PodCidr,
            prefix: "10.244.1.0/24".into(), attributes: attrs(),
        });
        i.advertise(Advertisement {
            name: "pods".into(), kind: AdvertisementKind::PodCidr,
            prefix: "10.244.2.0/24".into(), attributes: attrs(),
        });
        assert_eq!(i.advertisements().len(), 1);
        assert_eq!(i.advertisement("pods").unwrap().prefix, "10.244.2.0/24");
    }

    // ── Communities ──────────────────────────────────────────────────────────

    #[test]
    fn bgp_no_export_filters_cross_as_advertisement() {
        let (_c, tenant) = cilium_test_ctx!("pkg/bgpv1/manager/reconcile.go", "Community.NoExport", "tenant-bgp-ne");
        let mut i = instance(tenant);
        i.upsert_peer(PeerConfig::defaults("ext", ip(10, 0, 0, 2), 65001, 65000)); // cross-AS
        let mut a = attrs();
        a.communities = vec![COMMUNITY_NO_EXPORT];
        i.advertise(Advertisement {
            name: "internal".into(), kind: AdvertisementKind::ServiceCidr,
            prefix: "10.96.0.0/12".into(), attributes: a,
        });
        let r = i.effective_advertisements_for("ext").unwrap();
        assert!(r.is_empty());
    }

    #[test]
    fn bgp_no_export_keeps_intra_as_advertisement() {
        let (_c, tenant) = cilium_test_ctx!("pkg/bgpv1/manager/reconcile.go", "Community.NoExport.SameAs", "tenant-bgp-nesame");
        let mut i = instance(tenant);
        i.upsert_peer(peer(65000)); // same AS
        let mut a = attrs();
        a.communities = vec![COMMUNITY_NO_EXPORT];
        i.advertise(Advertisement {
            name: "internal".into(), kind: AdvertisementKind::ServiceCidr,
            prefix: "10.96.0.0/12".into(), attributes: a,
        });
        let r = i.effective_advertisements_for("peer-a").unwrap();
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn bgp_no_advertise_filters_all_peers() {
        let (_c, tenant) = cilium_test_ctx!("pkg/bgpv1/manager/reconcile.go", "Community.NoAdvertise", "tenant-bgp-na");
        let mut i = instance(tenant);
        i.upsert_peer(peer(65000));
        let mut a = attrs();
        a.communities = vec![COMMUNITY_NO_ADVERTISE];
        i.advertise(Advertisement {
            name: "secret".into(), kind: AdvertisementKind::ServiceCidr,
            prefix: "10.96.0.0/12".into(), attributes: a,
        });
        let r = i.effective_advertisements_for("peer-a").unwrap();
        assert!(r.is_empty());
    }

    // ── Path selection ───────────────────────────────────────────────────────

    #[test]
    fn bgp_path_selection_higher_local_pref_wins() {
        let (_c, tenant) = cilium_test_ctx!("pkg/bgpv1/manager/reconcile.go", "BestPath.LocalPref", "tenant-bgp-lp");
        let mut i = instance(tenant);
        let mut a = attrs(); a.local_pref = 100;
        let mut b = attrs(); b.local_pref = 200;
        i.record_incoming_path("10.0.0.0/24", "p1", a);
        i.record_incoming_path("10.0.0.0/24", "p2", b);
        let (peer_name, _) = i.best_path("10.0.0.0/24").unwrap();
        assert_eq!(peer_name, "p2");
    }

    #[test]
    fn bgp_path_selection_shorter_as_path_wins() {
        let (_c, tenant) = cilium_test_ctx!("pkg/bgpv1/manager/reconcile.go", "BestPath.AsPathLength", "tenant-bgp-aspath");
        let mut i = instance(tenant);
        let mut a = attrs(); a.as_path = vec![65001, 65002, 65003];
        let mut b = attrs(); b.as_path = vec![65010];
        i.record_incoming_path("10.0.0.0/24", "p1", a);
        i.record_incoming_path("10.0.0.0/24", "p2", b);
        let (peer_name, _) = i.best_path("10.0.0.0/24").unwrap();
        assert_eq!(peer_name, "p2");
    }

    #[test]
    fn bgp_path_selection_lower_origin_wins() {
        let (_c, tenant) = cilium_test_ctx!("pkg/bgpv1/manager/reconcile.go", "BestPath.Origin", "tenant-bgp-org");
        let mut i = instance(tenant);
        let mut a = attrs(); a.origin = PathOrigin::Egp;
        let mut b = attrs(); b.origin = PathOrigin::Igp;
        i.record_incoming_path("10.0.0.0/24", "p1", a);
        i.record_incoming_path("10.0.0.0/24", "p2", b);
        let (peer_name, _) = i.best_path("10.0.0.0/24").unwrap();
        assert_eq!(peer_name, "p2");
    }

    #[test]
    fn bgp_path_selection_lower_med_wins() {
        let (_c, tenant) = cilium_test_ctx!("pkg/bgpv1/manager/reconcile.go", "BestPath.MED", "tenant-bgp-med");
        let mut i = instance(tenant);
        let mut a = attrs(); a.med = 100;
        let mut b = attrs(); b.med = 50;
        i.record_incoming_path("10.0.0.0/24", "p1", a);
        i.record_incoming_path("10.0.0.0/24", "p2", b);
        let (peer_name, _) = i.best_path("10.0.0.0/24").unwrap();
        assert_eq!(peer_name, "p2");
    }

    #[test]
    fn bgp_best_path_no_candidates_returns_none() {
        let (_c, tenant) = cilium_test_ctx!("pkg/bgpv1/manager/reconcile.go", "BestPath.NoCandidates", "tenant-bgp-nc");
        let i = instance(tenant);
        assert!(i.best_path("10.0.0.0/24").is_none());
    }

    #[test]
    fn bgp_best_path_single_candidate_returns_it() {
        let (_c, tenant) = cilium_test_ctx!("pkg/bgpv1/manager/reconcile.go", "BestPath.Single", "tenant-bgp-single");
        let mut i = instance(tenant);
        i.record_incoming_path("10.0.0.0/24", "p1", attrs());
        let (peer_name, _) = i.best_path("10.0.0.0/24").unwrap();
        assert_eq!(peer_name, "p1");
    }

    #[test]
    fn bgp_paths_for_returns_all_recorded() {
        let (_c, tenant) = cilium_test_ctx!("pkg/bgpv1/manager/reconcile.go", "PathsFor", "tenant-bgp-pf");
        let mut i = instance(tenant);
        i.record_incoming_path("10.0.0.0/24", "p1", attrs());
        i.record_incoming_path("10.0.0.0/24", "p2", attrs());
        i.record_incoming_path("10.0.0.0/24", "p3", attrs());
        assert_eq!(i.paths_for("10.0.0.0/24").len(), 3);
    }

    // ── Lifecycle ────────────────────────────────────────────────────────────

    #[test]
    fn bgp_remove_peer_drops_status() {
        let (_c, tenant) = cilium_test_ctx!("pkg/bgpv1/manager/manager.go", "RemovePeer", "tenant-bgp-rmp");
        let mut i = instance(tenant);
        i.upsert_peer(peer(64999));
        i.remove_peer("peer-a").unwrap();
        assert!(i.peer_status("peer-a").is_none());
    }

    #[test]
    fn bgp_remove_unknown_peer_returns_not_found() {
        let (_c, tenant) = cilium_test_ctx!("pkg/bgpv1/manager/manager.go", "RemovePeer.NotFound", "tenant-bgp-rmpnf");
        let mut i = instance(tenant);
        let err = i.remove_peer("ghost").unwrap_err();
        assert!(matches!(err, BgpError::PeerNotFound(_)));
    }

    #[test]
    fn bgp_peer_count_tracks_upserts() {
        let (_c, tenant) = cilium_test_ctx!("pkg/bgpv1/manager/manager.go", "PeerCount", "tenant-bgp-cnt");
        let mut i = instance(tenant);
        for i_n in 0..5u8 {
            i.upsert_peer(PeerConfig::defaults(format!("p-{i_n}"), ip(10, 0, 0, i_n), 64999, 65000));
        }
        assert_eq!(i.peer_count(), 5);
    }

    // ── Serde ────────────────────────────────────────────────────────────────

    #[test]
    fn bgp_peer_config_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/k8s/apis/cilium.io/v2alpha1/bgp_types.go", "PeerConfig.Serde", "tenant-bgp-pserde");
        let p = peer(64999);
        let json = serde_json::to_string(&p).unwrap();
        let back: PeerConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn bgp_advertisement_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/k8s/apis/cilium.io/v2alpha1/bgp_types.go", "Advertisement.Serde", "tenant-bgp-aserde");
        let ad = Advertisement {
            name: "pods".into(), kind: AdvertisementKind::PodCidr,
            prefix: "10.244.1.0/24".into(), attributes: attrs(),
        };
        let json = serde_json::to_string(&ad).unwrap();
        let back: Advertisement = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ad);
    }

    #[test]
    fn bgp_path_attributes_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/bgpv1/types/types.go", "PathAttributes.Serde", "tenant-bgp-attrserde");
        let mut a = attrs();
        a.communities = vec![COMMUNITY_NO_EXPORT, 65001];
        let json = serde_json::to_string(&a).unwrap();
        let back: PathAttributes = serde_json::from_str(&json).unwrap();
        assert_eq!(back, a);
    }

    #[test]
    fn bgp_peer_state_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/bgpv1/types/types.go", "PeerState.Serde", "tenant-bgp-pstate");
        for s in [PeerState::Idle, PeerState::Connect, PeerState::Active, PeerState::OpenSent, PeerState::OpenConfirm, PeerState::Established] {
            let j = serde_json::to_string(&s).unwrap();
            let back: PeerState = serde_json::from_str(&j).unwrap();
            assert_eq!(back, s);
        }
    }
}

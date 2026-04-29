//! cave-net Cilium-parity end-to-end integration tests.
//!
//! Cross-module scenarios that exercise the public surface against
//! Cilium upstream semantics. Each test wires several agent-side
//! managers together (policy + ipcache + selector cache + LB +
//! conntrack + Hubble + clustermesh, etc.) and asserts the observable
//! behaviour matches what `cilium/cilium v1.19.3` produces for the
//! same input.

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr};

use cave_net::cilium::{
    arp_announce::{ArpAnnouncer, AnnounceProto},
    auth::{AuthMode, AuthVerdict, AuthManager, Svid},
    bandwidth::{BandwidthManager, CongestionControl, parse_bandwidth_annotation},
    bgp::{BgpInstance, PeerConfig as BgpPeerConfig, PeerEvent, PeerState},
    bpf_dump::{BpfMapKind, BpfMapRegistry, BpfDumpEntry},
    bpfmaps::{HashMapBpf, LruMap, LpmTrie},
    cilium_node::{CiliumNodeSpec, CiliumNodeStore, NodeAddress, AddressKind, NodeIpamSpec},
    cluster_pool_refill::{NodeWatermarkSpec, RefillController, RefillKind},
    clustermesh_ext::{KvStore, RemoteClusterStatus, RemoteClusterState, ServiceAffinity, GlobalService, GlobalServiceEndpoint},
    config_watcher::{ChangeAction, ConfigWatcher},
    conn_test::{ConnectivitySuite, ExpectedVerdict, ActualOutcome, Test as CtTest, TestKind},
    conntrack::{ConntrackTable, Direction as CtDirection, Tuple, TcpFlag, TcpState},
    dns_proxy::{DnsMode, DnsProxy, DnsQuestion, QType, AllowList, DnsRcode, DnsResponse, DnsAnswer, AnswerData, DnsVerdict},
    egress::{EgressGatewayPolicy, EgressManager, GatewayNode},
    endpoint::{EndpointManager, EndpointState, BpfProgram as EndpointBpfProgram, canonical_egress_chain},
    endpoint_regen::{RegenController, RegenLevel, RegenRequest},
    external_workload::{ExternalWorkloadManager, ExternalWorkloadSpec, WorkloadState},
    fqdn::{FqdnCache, match_pattern},
    hubble::{Verdict as HubbleVerdict, FlowLog, DropReason, TopologyGraph},
    hubble_ext::{Observer, GetFlowsRequest, FlowFilter, Relay},
    hubble_metrics::{MetricRegistry, MonitorEvent, NodeAggregator},
    id_coord::IdentityLockCoordinator,
    identity::{LabelSet, LocalIdentityCache, MIN_LOCAL_IDENTITY},
    ipam::{Ipam, IpamMode, PodIpPool},
    ipcache::{Ipcache, IpcacheEntry, IpcacheSource},
    k8s_handlers::{EndpointSlice, SliceEndpoint, SlicePort, EndpointCondition, EndpointSliceHandler, ServiceCidrSpec, ServiceCidrRegistry},
    key_rotation::{KeyRotationController, RotationPhase},
    kv_identity::{KvIdentityAllocator, MIN_GLOBAL_IDENTITY},
    l2_announce::{L2Announcer, L2AnnouncementPolicy, ServiceSelector, InterfaceMatcher, ServiceFrontends},
    lb::{Algorithm, Backend as LbBackend, LoadBalancer, FlowKey},
    lb_ext::{LbMode, KubeProxyReplacementMode, KubeProxyReplacementStatus, ServiceTrafficConfig, TrafficPolicy},
    lrp::{L4Proto, LocalBackend, LrpManager, make_node_local_dns_lrp},
    maglev::{MaglevTable, Backend as MaglevBackend, hash_5tuple, DEFAULT_M},
    maps_gc::{GcSweepReport, GcTarget, MapsGcController},
    nat::{DnatEntry, DnatTable, SnatKey, SnatTable},
    operator::{IdentityGc, CesManager, CiliumEndpoint},
    policy::{
        distill, Direction, EndpointSelector, IngressRule, InMemoryIdentityResolver, L4Protocol,
        PolicyEnforcementMode, PolicyRepository, PortProtocol, PortRule, Rule, Verdict as PolicyVerdict,
    },
    policy_trace::trace,
    proxy_health::{ProxyHealthChecker, ProxyProbe, ProxyState},
    readiness::{ReadinessGateController, GateStatus},
    recorder::{Recorder, RecorderPolicy, RecorderProto, RecorderTuple},
    reserved_ids::{ReservedIdentity, full_table, is_reserved_range},
    selector_cache::SelectorCache,
    services::{Service, ServiceRegistry, ServiceType},
    sock_lb::{CgroupHook, SockLbConfig, SockLbDecision, SockLbManager, ServiceFrontend, ServiceBackend},
    srv6::{Srv6Manager, Srv6Behavior, Locator, Sid},
    status::{ComponentName, ComponentStatus, DaemonState, StatusBoard},
    tunnel::{EncapDecision, TunnelEndpoint, TunnelManager, TunnelMode},
    types::TenantId,
    wireguard::{WgAgent, WgKey, WgMode, WgPeer},
};

fn ip4(a: u8, b: u8, c: u8, d: u8) -> IpAddr {
    IpAddr::V4(Ipv4Addr::new(a, b, c, d))
}

fn ls(pairs: &[(&str, &str)]) -> LabelSet {
    LabelSet::from_iter(pairs.iter().map(|(k, v)| (*k, *v)))
}

fn endpoint_sel(pairs: &[(&str, &str)]) -> EndpointSelector {
    EndpointSelector {
        match_labels: pairs.iter().map(|(k, v)| ((*k).into(), (*v).into())).collect(),
        match_expressions: Vec::new(),
    }
}

// ── E2E #1: Identity → Policy → Verdict round-trip ──────────────────────────

#[test]
fn e2e_identity_allocation_drives_policy_verdict() {
    let tenant = TenantId::new("e2e-1");
    let mut cache = LocalIdentityCache::new(tenant.clone());
    let client_id = cache.lookup_or_allocate(&ls(&[("app", "client")])).unwrap();
    let web_id = cache.lookup_or_allocate(&ls(&[("app", "web")])).unwrap();
    assert_ne!(client_id, web_id);
    assert!(client_id >= MIN_LOCAL_IDENTITY);

    let mut repo = PolicyRepository::new();
    let mut rule = Rule::new("allow-client-to-web", tenant.clone(), endpoint_sel(&[("app", "web")]));
    rule.ingress.push(IngressRule {
        from_endpoints: vec![endpoint_sel(&[("app", "client")])],
        to_ports: vec![PortRule {
            ports: vec![PortProtocol::new(80, L4Protocol::TCP)],
            l7_redirect_port: None,
        }],
        ..Default::default()
    });
    repo.add(rule);

    let mut resolver = InMemoryIdentityResolver::new();
    resolver.insert(client_id, ls(&[("app", "client")]));
    resolver.insert(web_id, ls(&[("app", "web")]));
    let map = distill(&repo, &tenant, &ls(&[("app", "web")]),
        PolicyEnforcementMode::Default, &resolver).unwrap();

    assert!(map.ingress_enforced);
    assert_eq!(map.lookup(client_id, 80, L4Protocol::TCP, Direction::Ingress).verdict, PolicyVerdict::Allow);
    assert_eq!(map.lookup(client_id, 8080, L4Protocol::TCP, Direction::Ingress).verdict, PolicyVerdict::Deny);
    assert_eq!(map.lookup(web_id, 80, L4Protocol::TCP, Direction::Ingress).verdict, PolicyVerdict::Deny);
}

// ── E2E #2: Service registration → LB → conntrack ──────────────────────────

#[test]
fn e2e_service_lb_and_conntrack_round_trip() {
    let tenant = TenantId::new("e2e-2");
    let mut reg = ServiceRegistry::new();
    let mut svc = Service::cluster_ip("api", "default", tenant.clone(), ip4(10, 96, 0, 1), 80);
    svc.algorithm = Algorithm::RoundRobin;
    svc.backends = vec![
        LbBackend::new("a", ip4(10, 0, 1, 1), 8080),
        LbBackend::new("b", ip4(10, 0, 1, 2), 8080),
    ];
    reg.upsert(svc).unwrap();

    let lb = reg.lb_for("default/api", "default").unwrap();
    let key = FlowKey { src_ip: ip4(10, 0, 0, 5), src_port: 1234, dst_ip: ip4(10, 96, 0, 1), dst_port: 80, proto: 6 };
    let backend = lb.select(key, 100).unwrap().clone();
    assert!(["a", "b"].contains(&backend.name.as_str()));

    let mut ct = ConntrackTable::new(tenant, 1024);
    let tup = Tuple::new(ip4(10, 0, 0, 5), 1234, ip4(10, 96, 0, 1), 80, L4Protocol::TCP);
    ct.upsert(tup, CtDirection::Egress, 100, 64, Some(1));
    ct.apply_tcp_flag(tup, CtDirection::Egress, TcpFlag::SynAck, 110);
    ct.apply_tcp_flag(tup, CtDirection::Egress, TcpFlag::Ack, 111);
    let entry = ct.lookup(tup, CtDirection::Egress).unwrap();
    assert_eq!(entry.tcp_state, Some(TcpState::Established));
    assert_eq!(entry.rev_nat_index, Some(1));
}

// ── E2E #3: Maglev consistency under backend churn ─────────────────────────

#[test]
fn e2e_maglev_consistent_under_backend_addition() {
    let tenant = TenantId::new("e2e-3");
    let backs_before: Vec<MaglevBackend> = (0..5).map(|i| MaglevBackend::new(format!("b-{i}"), 1)).collect();
    let backs_after: Vec<MaglevBackend> = (0..6).map(|i| MaglevBackend::new(format!("b-{i}"), 1)).collect();
    let m = 1009usize;
    let before = MaglevTable::build(tenant.clone(), m, backs_before).unwrap();
    let after = MaglevTable::build(tenant, m, backs_after).unwrap();

    let mut moved = 0usize;
    for slot in 0..m {
        let b_name = &before.backends[before.lookup[slot] as usize].name;
        let a_name = &after.backends[after.lookup[slot] as usize].name;
        if b_name != a_name {
            moved += 1;
        }
    }
    // Adding 1 of 6 backends — expect ~m/6 disruption, allow 50% upper bound.
    assert!(moved <= m / 2, "moved={moved} m={m}");
}

#[test]
fn e2e_maglev_default_table_size_is_prime() {
    assert_eq!(DEFAULT_M, 16381);
    let tenant = TenantId::new("e2e-3b");
    let backs = vec![MaglevBackend::new("only", 1)];
    let table = MaglevTable::build(tenant, DEFAULT_M, backs).unwrap();
    let h = hash_5tuple(0x0a000001, 0x0a600001, 1234, 80, 6);
    assert_eq!(table.lookup(h).name, "only");
}

// ── E2E #4: NAT round-trip (SNAT alloc → DNAT install → reverse lookup) ────

#[test]
fn e2e_nat_snat_dnat_round_trip() {
    let tenant = TenantId::new("e2e-4");
    let mut snat = SnatTable::new(tenant.clone(), ip4(192, 168, 1, 1));
    let key = SnatKey { src_ip: ip4(10, 0, 0, 1), src_port: 1234, dst_ip: ip4(8, 8, 8, 8), dst_port: 53 };
    let entry = snat.allocate(key, 100).unwrap();
    assert_eq!(entry.new_src_ip, ip4(192, 168, 1, 1));
    let rev_key = SnatKey { src_ip: entry.new_src_ip, src_port: entry.new_src_port, dst_ip: key.dst_ip, dst_port: key.dst_port };
    assert_eq!(snat.lookup_reverse(&rev_key), Some(key));

    let mut dnat = DnatTable::new(tenant);
    let backend = DnatEntry { backend_ip: ip4(10, 0, 1, 5), backend_port: 8080 };
    let idx = dnat.install(backend).unwrap();
    assert_eq!(dnat.lookup(idx), Some(backend));
}

// ── E2E #5: IPAM allocate → release ──────────────────────────────────────

#[test]
fn e2e_ipam_cluster_pool_idempotent_per_owner() {
    let tenant = TenantId::new("e2e-5");
    let mut ipam = Ipam::new(IpamMode::ClusterPool);
    ipam.upsert_pool(PodIpPool::ipv4("default", tenant, "10.0.0.0/24")).unwrap();
    let ip_a = ipam.allocate_v4("default", "ns/p1").unwrap();
    let ip_b = ipam.allocate_v4("default", "ns/p1").unwrap();
    assert_eq!(ip_a, ip_b, "same owner must receive same IP (idempotent)");
    let ip_c = ipam.allocate_v4("default", "ns/p2").unwrap();
    assert_ne!(ip_a, ip_c);
    assert_eq!(ipam.release_owner("ns/p1"), 1);
}

// ── E2E #6: IPCache source-priority ────────────────────────────────────────

#[test]
fn e2e_ipcache_source_priority_resolution() {
    let tenant = TenantId::new("e2e-6");
    let mut cache = Ipcache::new(tenant);
    let kv_entry = IpcacheEntry { identity: 999, source: IpcacheSource::Kvstore, encryption_key: 0, tunnel_endpoint: None };
    let local_entry = IpcacheEntry { identity: 256, source: IpcacheSource::Local, encryption_key: 0, tunnel_endpoint: None };
    cache.upsert(ip4(10, 0, 1, 5), kv_entry).unwrap();
    cache.upsert(ip4(10, 0, 1, 5), local_entry).unwrap();
    assert_eq!(cache.identity_of(ip4(10, 0, 1, 5)), Some(256));
    let kv2 = IpcacheEntry { identity: 1234, source: IpcacheSource::Kvstore, encryption_key: 0, tunnel_endpoint: None };
    assert!(cache.upsert(ip4(10, 0, 1, 5), kv2).is_err());
    assert_eq!(cache.identity_of(ip4(10, 0, 1, 5)), Some(256));
}

// ── E2E #7: Hubble Observer → flow filter → Relay aggregation ──────────────

#[test]
fn e2e_hubble_observer_relay_aggregation() {
    let tenant = TenantId::new("e2e-7");
    let mut a = Observer::new(tenant.clone(), 100, 0);
    let mut b = Observer::new(tenant.clone(), 100, 0);
    let make_flow = |src_ns: &str, verdict: HubbleVerdict| FlowLog {
        tenant: tenant.clone(),
        time: chrono::Utc::now(),
        source_identity: 256,
        destination_identity: 257,
        source_pod: format!("{src_ns}/client"),
        destination_pod: "default/api".into(),
        verdict,
        drop_reason: DropReason::None,
        bytes: 100,
    };
    a.ingest(make_flow("ns-a", HubbleVerdict::Forwarded));
    a.ingest(make_flow("ns-a", HubbleVerdict::Dropped));
    b.ingest(make_flow("ns-b", HubbleVerdict::Forwarded));

    let mut relay = Relay::new(tenant);
    relay.add_cluster("us-east", a);
    relay.add_cluster("eu-west", b);

    let req = GetFlowsRequest {
        whitelist: vec![FlowFilter { verdict: Some(HubbleVerdict::Dropped), ..Default::default() }],
        ..Default::default()
    };
    let dropped = relay.get_flows(&req);
    assert_eq!(dropped.len(), 1);
    assert!(dropped[0].source_pod.starts_with("ns-a/"));

    let all = relay.get_flows(&GetFlowsRequest::default());
    assert_eq!(all.len(), 3);
}

// ── E2E #8: Auth handshake — SVID register → handshake → resolve ───────────

#[test]
fn e2e_auth_full_mtls_handshake() {
    let tenant = TenantId::new("e2e-8");
    let mut m = AuthManager::new(tenant, "cluster.local");
    let svid_a = Svid::new("spiffe://cluster.local/client", "cluster.local", 100, 3600);
    let svid_b = Svid::new("spiffe://cluster.local/api", "cluster.local", 100, 3600);
    m.register_svid(256, svid_a).unwrap();
    m.register_svid(257, svid_b).unwrap();

    assert_eq!(m.resolve(256, 257, AuthMode::Required, 200), AuthVerdict::NeedsAuth);
    let entry = m.handshake(256, 257, AuthMode::Required, 200).unwrap();
    assert_eq!(entry.src_spiffe, "spiffe://cluster.local/client");
    assert_eq!(m.resolve(256, 257, AuthMode::Required, 300), AuthVerdict::Authorized);
    assert_eq!(m.resolve(257, 256, AuthMode::Required, 300), AuthVerdict::NeedsAuth);
    assert!(m.revoke_svid(256));
    assert_eq!(m.auth_count(), 0);
}

// ── E2E #9: BGP peer FSM full handshake ────────────────────────────────────

#[test]
fn e2e_bgp_full_session_lifecycle() {
    let tenant = TenantId::new("e2e-9");
    let mut bgp = BgpInstance::new(tenant, 65000, ip4(10, 0, 0, 1));
    bgp.upsert_peer(BgpPeerConfig::defaults("ext", ip4(10, 0, 0, 2), 64999, 65000));
    assert_eq!(bgp.peer_status("ext").unwrap().state, PeerState::Idle);
    bgp.handle_event("ext", PeerEvent::Start).unwrap();
    bgp.handle_event("ext", PeerEvent::TcpConnected).unwrap();
    bgp.handle_event("ext", PeerEvent::OpenReceived).unwrap();
    let s = bgp.handle_event("ext", PeerEvent::KeepaliveReceived).unwrap();
    assert_eq!(s, PeerState::Established);
    let s2 = bgp.handle_event("ext", PeerEvent::NotificationReceived).unwrap();
    assert_eq!(s2, PeerState::Idle);
}

// ── E2E #10: L2 announcer lease + ARP burst ────────────────────────────────

#[test]
fn e2e_l2_announcer_with_arp_burst() {
    let tenant = TenantId::new("e2e-10");
    let mut l2 = L2Announcer::new(tenant.clone(), "node-a", 60);
    l2.upsert_policy(L2AnnouncementPolicy {
        name: "lb".into(), tenant: tenant.clone(),
        service_selector: ServiceSelector { match_labels: vec![("type".into(), "public".into())] },
        interfaces: InterfaceMatcher { patterns: vec!["eth*".into()] },
        load_balancer_ips: true, external_ips: false,
    });
    l2.upsert_service(ServiceFrontends {
        key: "ns/svc".into(),
        labels: vec![("type".into(), "public".into())],
        load_balancer_ips: vec![ip4(203, 0, 113, 5)],
        external_ips: vec![],
        has_active_backends: true,
    });
    let announceable = l2.announceable(&["eth0".into()]);
    assert!(announceable.contains(&(ip4(203, 0, 113, 5), "eth0".into())));
    assert!(l2.try_acquire(ip4(203, 0, 113, 5), "eth0", 0));

    let mut arp = ArpAnnouncer::new(tenant, [0x02, 0, 0, 0, 0, 0xAA], 1);
    arp.enable_interface("eth0");
    arp.register_vip(ip4(203, 0, 113, 5));
    let n = arp.announce_burst(ip4(203, 0, 113, 5), "eth0", 5, 0).unwrap();
    assert_eq!(n, 5);
    assert_eq!(arp.sent_count(), 5);
    let frame = &arp.drain_sent()[0];
    assert_eq!(frame.proto, AnnounceProto::GratuitousArp);
}

// ── E2E #11: Tunnel encap decision under native-routing CIDR ──────────────

#[test]
fn e2e_tunnel_native_routing_overrides_endpoint() {
    let tenant = TenantId::new("e2e-11");
    let mut m = TunnelManager::new(tenant, TunnelMode::Vxlan, Some("10.244.0.0/16".into())).unwrap();
    m.upsert_endpoint(TunnelEndpoint {
        node_name: "node-a".into(),
        node_ip: ip4(10, 0, 0, 2),
        pod_cidr: "10.244.1.0/24".into(),
        vni: 0,
    }).unwrap();
    assert_eq!(m.lookup_encap(ip4(10, 244, 1, 5)).unwrap(), EncapDecision::Native);
    assert_eq!(m.lookup_encap(ip4(192, 168, 1, 1)).unwrap(), EncapDecision::Unknown);
}

// ── E2E #12: SocketLB connect/recv reverse round-trip ──────────────────────

#[test]
fn e2e_sock_lb_connect_recv_roundtrip() {
    let tenant = TenantId::new("e2e-12");
    let mut m = SockLbManager::new(tenant, SockLbConfig::default());
    let id = m.register_service(
        ServiceFrontend { cluster_ip: ip4(10, 96, 0, 1), port: 80, protocol: 6 },
        vec![ServiceBackend { backend_id: 1, backend_ip: ip4(10, 0, 1, 1), backend_port: 8080 }],
    ).unwrap();
    let d = m.on_connect(ip4(10, 96, 0, 1), 80, 6, false, 0);
    match d {
        SockLbDecision::Rewrite { revnat_id, backend_ip, .. } => {
            assert_eq!(revnat_id, id);
            assert_eq!(backend_ip, ip4(10, 0, 1, 1));
        }
        _ => panic!("expected Rewrite"),
    }
    let (orig_ip, orig_port) = m.on_recvmsg(id).unwrap();
    assert_eq!(orig_ip, ip4(10, 96, 0, 1));
    assert_eq!(orig_port, 80);
}

// ── E2E #13: ClusterMesh KVStore prefix watch ──────────────────────────────

#[test]
fn e2e_clustermesh_kvstore_watch() {
    let tenant = TenantId::new("e2e-13");
    let mut kv = KvStore::new(tenant);
    let w = kv.watch("/cilium/identities/");
    kv.put("/cilium/identities/256", b"client".to_vec(), None);
    kv.put("/cilium/services/foo", b"bar".to_vec(), None);
    kv.delete("/cilium/identities/256");
    let events = kv.drain_watch(w);
    assert_eq!(events.len(), 2);
}

// ── E2E #14: ClusterMesh global service affinity fallback ──────────────────

#[test]
fn e2e_clustermesh_local_affinity_falls_through_to_remote() {
    let mut svc = GlobalService::new("api", "default", ServiceAffinity::Local);
    svc.remote_endpoints.push(GlobalServiceEndpoint { cluster: "us-east".into(), address: "10.0.2.1".into(), port: 80 });
    let r = svc.resolve();
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].cluster, "us-east");
}

// ── E2E #15: Egress gateway HA failover ────────────────────────────────────

#[test]
fn e2e_egress_gateway_ha_failover() {
    let tenant = TenantId::new("e2e-15");
    let mut mgr = EgressManager::new();
    let mut p = EgressGatewayPolicy::new(
        "egw", tenant,
        endpoint_sel(&[("app", "billing")]),
        ip4(192, 0, 2, 100),
    );
    p.destination_cidrs = vec!["1.0.0.0/8".into()];
    p.gateway_nodes = vec![
        GatewayNode { name: "gw1".into(), node_ip: ip4(10, 0, 0, 1), state: cave_net::cilium::egress::GatewayState::Unhealthy },
        GatewayNode::new("gw2", ip4(10, 0, 0, 2)),
    ];
    mgr.upsert(p).unwrap();
    for h in 0..10u64 {
        let dec = mgr.evaluate(&ls(&[("app", "billing")]), &ls(&[]), ip4(1, 1, 1, 1), h).unwrap().unwrap();
        assert_eq!(dec.gateway_node, "gw2");
    }
}

// ── E2E #16: LRP node-local DNS happy path ────────────────────────────────

#[test]
fn e2e_lrp_node_local_dns_redirect() {
    let tenant = TenantId::new("e2e-16");
    let mut m = LrpManager::new();
    m.upsert_policy(make_node_local_dns_lrp(tenant)).unwrap();
    m.upsert_backend(LocalBackend {
        pod_name: "nodelocaldns-node-a".into(),
        pod_namespace: "kube-system".into(),
        node_name: "node-a".into(),
        pod_ip: ip4(169, 254, 20, 10),
        labels: vec![("k8s-app".into(), "node-local-dns".into())],
    });
    let r = m.resolve("node-a", ip4(10, 96, 0, 10), 53, L4Proto::UDP, Some(("kube-system", "kube-dns"))).unwrap();
    assert_eq!(r.target_port, 53);
    assert_eq!(r.backend_ip, ip4(169, 254, 20, 10));
}

// ── E2E #17: DNS proxy intercept → FQDN cache update ──────────────────────

#[test]
fn e2e_dns_proxy_intercept_updates_resolved() {
    let tenant = TenantId::new("e2e-17");
    let mut p = DnsProxy::new(tenant.clone(), DnsMode::Intercept);
    p.set_allow_list(1, AllowList { patterns: vec!["*.example.com".into()] });
    let q = DnsQuestion { qname: "api.example.com".into(), qtype: QType::A };
    let v = p.on_query(1, &q, 100).unwrap();
    assert_eq!(v, DnsVerdict::Allow);
    let resp = DnsResponse {
        rcode: DnsRcode::NoError,
        answers: vec![DnsAnswer {
            name: "api.example.com".into(), qtype: QType::A,
            ttl_seconds: 60, data: AnswerData::A(Ipv4Addr::new(1, 2, 3, 4)),
        }],
    };
    p.on_response(&q, &resp, 200);
    let resolved = p.lookup_resolved("api.example.com", 200);
    assert_eq!(resolved, vec![ip4(1, 2, 3, 4)]);

    let mut fc = FqdnCache::new(tenant);
    let id = fc.resolve("*.example.com", "api.example.com", ip4(1, 2, 3, 4), 60, 200).unwrap();
    assert!(id >= 16_777_216);
}

// ── E2E #18: WireGuard peer registration + key serialization ──────────────

#[test]
fn e2e_wireguard_keypair_and_peer_registry() {
    let tenant = TenantId::new("e2e-18");
    let mut a = WgAgent::new(tenant, "node-a", WgMode::PerNode, 1);
    assert_ne!(a.private_key, a.public_key);
    let s = a.public_key.to_base64();
    assert_eq!(s.len(), 44);
    let back = WgKey::from_base64(&s).unwrap();
    assert_eq!(back, a.public_key);
    a.upsert_peer(WgPeer {
        node: "node-b".into(),
        public_key: WgKey::from_seed(99),
        endpoint: "10.0.0.2:51820".parse().unwrap(),
        allowed_ips: vec!["10.244.1.0/24".into()],
        psk: None,
    });
    assert_eq!(a.peer_count(), 1);
}

// ── E2E #19: Encryption key rotation drain → switched ─────────────────────

#[test]
fn e2e_key_rotation_drain_then_switch() {
    let tenant = TenantId::new("e2e-19");
    let mut c = KeyRotationController::new(tenant, 30, 100);
    c.install_initial("node-a", vec![1, 2, 3], 0);
    let new_spi = c.begin_rotation("node-a", vec![4, 5, 6], 100).unwrap();
    assert!(new_spi > 100);
    assert_eq!(c.phase("node-a"), Some(RotationPhase::Drain));
    assert!(c.complete_rotation("node-a", 100 + 10_000_000_000).is_err());
    c.complete_rotation("node-a", 100 + 31_000_000_000).unwrap();
    assert_eq!(c.phase("node-a"), Some(RotationPhase::Switched));
    assert!(c.previous_spi("node-a").is_none());
}

// ── E2E #20: kube-proxy-replacement traffic policy ─────────────────────────

#[test]
fn e2e_kpr_traffic_policy_and_source_range() {
    let s = KubeProxyReplacementStatus::strict_default();
    assert!(matches!(s.mode, KubeProxyReplacementMode::Strict));
    assert!(matches!(s.lb_mode, LbMode::Hybrid));

    let cfg = ServiceTrafficConfig {
        internal: TrafficPolicy::Cluster,
        external: TrafficPolicy::Local,
        source_ranges: vec!["10.0.0.0/8".into()],
    };
    assert!(cfg.allows_source(ip4(10, 1, 1, 1)).unwrap());
    assert!(!cfg.allows_source(ip4(192, 168, 1, 1)).unwrap());
}

// ── E2E #21: SRv6 SID list + locator longest-prefix ──────────────────────

#[test]
fn e2e_srv6_locator_longest_prefix() {
    let tenant = TenantId::new("e2e-21");
    let mut m = Srv6Manager::new(tenant);
    m.upsert_locator(Locator {
        prefix: "fd00::".parse().unwrap(),
        prefix_len: 16,
        behavior: Srv6Behavior::End,
    });
    m.upsert_locator(Locator {
        prefix: "fd00:db8::".parse().unwrap(),
        prefix_len: 32,
        behavior: Srv6Behavior::EndDt4 { vrf_id: 7 },
    });
    let l = m.lookup_locator(Sid("fd00:db8::1234".parse().unwrap())).unwrap();
    assert!(matches!(l.behavior, Srv6Behavior::EndDt4 { vrf_id: 7 }));
}

// ── E2E #22: BGP NO_EXPORT filters cross-AS advertisement ────────────────

#[test]
fn e2e_bgp_no_export_filter() {
    use cave_net::cilium::bgp::{Advertisement, AdvertisementKind, PathAttributes, PathOrigin, COMMUNITY_NO_EXPORT};
    let tenant = TenantId::new("e2e-22");
    let mut bgp = BgpInstance::new(tenant, 65000, ip4(10, 0, 0, 1));
    bgp.upsert_peer(BgpPeerConfig::defaults("ext", ip4(10, 0, 0, 2), 65001, 65000));
    let attrs = PathAttributes {
        origin: PathOrigin::Igp, as_path: vec![65000], next_hop: ip4(10, 0, 0, 1),
        local_pref: 100, med: 0, communities: vec![COMMUNITY_NO_EXPORT],
    };
    bgp.advertise(Advertisement {
        name: "internal".into(),
        kind: AdvertisementKind::ServiceCidr,
        prefix: "10.96.0.0/12".into(),
        attributes: attrs,
    });
    let r = bgp.effective_advertisements_for("ext").unwrap();
    assert!(r.is_empty(), "NO_EXPORT must filter cross-AS advertisement");
    bgp.upsert_peer(BgpPeerConfig::defaults("intra", ip4(10, 0, 0, 3), 65000, 65000));
    let r2 = bgp.effective_advertisements_for("intra").unwrap();
    assert_eq!(r2.len(), 1);
}

// ── E2E #23: Hubble metrics — drop counter aggregation ─────────────────────

#[test]
fn e2e_hubble_metrics_drop_aggregation() {
    let tenant = TenantId::new("e2e-23");
    let mut r = MetricRegistry::new();
    let make_flow = |verdict: HubbleVerdict, reason: DropReason| FlowLog {
        tenant: tenant.clone(),
        time: chrono::Utc::now(),
        source_identity: 256, destination_identity: 257,
        source_pod: "ns/a".into(), destination_pod: "ns/b".into(),
        verdict, drop_reason: reason, bytes: 100,
    };
    r.process_flow(&make_flow(HubbleVerdict::Forwarded, DropReason::None));
    r.process_flow(&make_flow(HubbleVerdict::Dropped, DropReason::PolicyDeny));
    r.process_flow(&make_flow(HubbleVerdict::Dropped, DropReason::PolicyDeny));
    let samples = r.samples();
    let drop_count: u64 = samples.iter()
        .filter(|s| s.name == "hubble_drop_total")
        .map(|s| s.value as u64)
        .sum();
    assert_eq!(drop_count, 2);
    assert!(MonitorEvent::from_numeric(7) == Some(MonitorEvent::PolicyVerdict));
}

// ── E2E #24: Operator Identity GC ──────────────────────────────────────────

#[test]
fn e2e_operator_identity_gc_sweeps_unreferenced() {
    let tenant = TenantId::new("e2e-24");
    let mut gc = IdentityGc::new(tenant, MIN_LOCAL_IDENTITY, 60);
    gc.add_reference(256, 0).unwrap();
    gc.release_reference(256, 0).unwrap();
    let report = gc.sweep(60_000_000_000 + 1);
    assert_eq!(report.deleted, 1);
    assert_eq!(gc.tracked_count(), 0);
}

// ── E2E #25: Operator CES batches endpoints ──────────────────────────────

#[test]
fn e2e_operator_ces_batching() {
    let tenant = TenantId::new("e2e-25");
    let mut m = CesManager::new(tenant, 3);
    for i in 0..7u32 {
        m.upsert(CiliumEndpoint {
            name: format!("p-{i}"), namespace: "default".into(),
            identity: 256 + i, pod_name: format!("p-{i}"),
        });
    }
    assert_eq!(m.endpoint_count(), 7);
    assert_eq!(m.ces_count(), 3);
}

// ── E2E #26: SelectorCache identity update propagates change ─────────────

#[test]
fn e2e_selector_cache_membership_update() {
    let tenant = TenantId::new("e2e-26");
    let mut sc = SelectorCache::new(tenant);
    let sid = sc.intern(endpoint_sel(&[("app", "web")]));
    sc.update_identity(256, ls(&[("app", "web")]));
    let added = sc.drain_changes();
    assert_eq!(added.len(), 1);
    assert!(added[0].added.contains(&256));

    sc.update_identity(256, ls(&[("app", "api")]));
    let removed = sc.drain_changes();
    assert_eq!(removed.len(), 1);
    assert!(removed[0].removed.contains(&256));
    assert_eq!(removed[0].selector_id, sid);
}

// ── E2E #27: KVStore-backed identity allocator + lock coordinator ────────

#[test]
fn e2e_kv_identity_with_lock_coordinator() {
    let tenant = TenantId::new("e2e-27");
    let mut alloc = KvIdentityAllocator::new(tenant.clone(), 60);
    let mut lock = IdentityLockCoordinator::new(tenant, 30);
    let labels = ls(&[("app", "web"), ("env", "prod")]);
    let key = "/cilium/identities/labels/{app=web,env=prod}";

    lock.try_lock(key, "agent-a", 0).unwrap();
    let id_a = alloc.allocate(&labels, 0).unwrap();
    assert!(id_a >= MIN_GLOBAL_IDENTITY);
    assert!(lock.try_lock(key, "agent-b", 1_000_000_000).is_err());
    lock.release(key, "agent-a").unwrap();
    lock.try_lock(key, "agent-b", 5_000_000_000).unwrap();
    let id_b = alloc.allocate(&labels, 5_000_000_000).unwrap();
    assert_eq!(id_a, id_b);
}

// ── E2E #28: Reserved identity table integrity ───────────────────────────

#[test]
fn e2e_reserved_identity_table() {
    let table = full_table();
    assert_eq!(table.get(&1).copied(), Some(ReservedIdentity::Host));
    assert_eq!(table.get(&2).copied(), Some(ReservedIdentity::World));
    assert_eq!(table.get(&7).copied(), Some(ReservedIdentity::KubeApiServer));
    assert_eq!(table.get(&8).copied(), Some(ReservedIdentity::Ingress));
    assert!(is_reserved_range(255));
    assert!(!is_reserved_range(256));
    assert_eq!(MIN_LOCAL_IDENTITY, 256);
}

// ── E2E #29: Cluster-pool refill controller end-to-end ────────────────────

#[test]
fn e2e_cluster_pool_refill_lifecycle() {
    let tenant = TenantId::new("e2e-29");
    let mut c = RefillController::new(tenant);
    c.seed_pool(vec!["10.244.0.0/24".into(), "10.244.1.0/24".into(), "10.244.2.0/24".into()]);
    c.upsert_node(NodeWatermarkSpec {
        node: "node-a".into(),
        pre_allocate: 8, max_above_watermark: 16,
        allocated: vec![], used_ips: 0, capacity_per_subnet: 256,
    });
    let actions = c.reconcile();
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0].kind, RefillKind::Add);
    assert_eq!(c.pool_remaining(), 2);
}

// ── E2E #30: Bandwidth manager + EDT ─────────────────────────────────────

#[test]
fn e2e_bandwidth_edt_paces_packets() {
    assert_eq!(parse_bandwidth_annotation("10M").unwrap(), 10_000_000);
    assert_eq!(parse_bandwidth_annotation("1Gi").unwrap(), 1024 * 1024 * 1024);

    let tenant = TenantId::new("e2e-30");
    let mut m = BandwidthManager::new(tenant, CongestionControl::Bbr);
    m.set_bandwidth(7, 1_000);
    let now = 0u64;
    let edt0 = m.schedule(7, now, 200);
    let edt1 = m.schedule(7, now, 1500);
    let edt2 = m.schedule(7, now, 1500);
    assert_eq!(edt0, 0);
    assert!(edt1 > now);
    assert!(edt2 >= edt1);
    assert!(m.bbr_enabled());
}

// ── E2E #31: PolicyTrace explains an Allow verdict ───────────────────────

#[test]
fn e2e_policy_trace_explains_allow() {
    let tenant = TenantId::new("e2e-31");
    let mut repo = PolicyRepository::new();
    let mut rule = Rule::new("allow-client", tenant.clone(), endpoint_sel(&[("app", "web")]));
    rule.ingress.push(IngressRule {
        from_endpoints: vec![endpoint_sel(&[("app", "client")])],
        to_ports: vec![PortRule { ports: vec![PortProtocol::new(80, L4Protocol::TCP)], l7_redirect_port: None }],
        ..Default::default()
    });
    repo.add(rule);
    let mut resolver = InMemoryIdentityResolver::new();
    resolver.insert(MIN_LOCAL_IDENTITY, ls(&[("app", "client")]));
    let map = distill(&repo, &tenant, &ls(&[("app", "web")]),
        PolicyEnforcementMode::Default, &resolver).unwrap();
    let t = trace(&tenant, &repo, &map, MIN_LOCAL_IDENTITY, 999, 80, L4Protocol::TCP, Direction::Ingress).unwrap();
    assert_eq!(t.final_verdict, PolicyVerdict::Allow);
    assert!(t.enforcement);
    assert_eq!(t.steps.len(), 1);
    assert_eq!(t.steps[0].rule_name, "allow-client");
}

// ── E2E #32: ConfigWatcher restart-required surfaces correctly ───────────

#[test]
fn e2e_config_watcher_restart_required_signal() {
    let tenant = TenantId::new("e2e-32");
    let mut w = ConfigWatcher::new(tenant);
    let mut cfg = BTreeMap::new();
    cfg.insert("enable-ipsec".into(), "false".into());
    cfg.insert("debug".into(), "false".into());
    w.update_cluster_config(cfg.clone());
    let _ = w.drain_changes();

    cfg.insert("enable-ipsec".into(), "true".into());
    cfg.insert("debug".into(), "true".into());
    w.update_cluster_config(cfg);
    let changes = w.drain_changes();
    let ipsec_change = changes.iter().find(|c| c.key == "enable-ipsec").unwrap();
    let debug_change = changes.iter().find(|c| c.key == "debug").unwrap();
    assert_eq!(ipsec_change.action, ChangeAction::RequireRestart);
    assert_eq!(debug_change.action, ChangeAction::Reconfigure);
}

// ── E2E #33: K8s EndpointSlice → ready backends ───────────────────────────

#[test]
fn e2e_k8s_endpoint_slice_ready_backends() {
    let mut h = EndpointSliceHandler::new();
    let mut slice = EndpointSlice {
        name: "api-abc".into(), namespace: "default".into(),
        service_name: "api".into(), address_type: "IPv4".into(),
        endpoints: vec![SliceEndpoint {
            addresses: vec![ip4(10, 0, 1, 1), ip4(10, 0, 1, 2)],
            condition: EndpointCondition::Ready,
            node_name: Some("node-a".into()),
            target_ref: Some("default/p1".into()), zone: None,
        }],
        ports: vec![SlicePort { name: "http".into(), port: 80, protocol: "TCP".into() }],
    };
    h.upsert(slice.clone());
    let ready = h.ready_backends("default", "api");
    assert_eq!(ready.len(), 2);
    assert!(ready.contains(&(ip4(10, 0, 1, 1), 80)));

    slice.endpoints[0].condition = EndpointCondition::Terminating;
    h.upsert(slice);
    let ready_after = h.ready_backends("default", "api");
    assert!(ready_after.is_empty());

    let mut sc = ServiceCidrRegistry::new();
    sc.upsert(ServiceCidrSpec { name: "default".into(), cidrs: vec!["10.96.0.0/12".into()] }).unwrap();
    assert!(sc.contains(ip4(10, 96, 0, 1)).unwrap());
}

// ── E2E #34: BPF map registry lifecycle ──────────────────────────────────

#[test]
fn e2e_bpf_map_registry_dump_lifecycle() {
    let tenant = TenantId::new("e2e-34");
    let mut r = BpfMapRegistry::new(tenant);
    r.register("cilium_ipcache", BpfMapKind::Ipcache, 65536);
    r.upsert_entry("cilium_ipcache",
        BpfDumpEntry { key_pretty: "10.0.1.5".into(), value_pretty: "id=256 key=0".into() }).unwrap();
    r.upsert_entry("cilium_ipcache",
        BpfDumpEntry { key_pretty: "10.0.1.5".into(), value_pretty: "id=999 key=0".into() }).unwrap();
    let dump = r.dump("cilium_ipcache").unwrap();
    assert_eq!(dump.entries.len(), 1);
    assert_eq!(dump.entries[0].value_pretty, "id=999 key=0");
    let metric = r.fill_metric("cilium_ipcache").unwrap();
    assert_eq!(metric.used, 1);
    assert_eq!(metric.capacity, 65536);
}

// ── E2E #35: BPF generic map types (Hash/LRU/LpmTrie) ─────────────────────

#[test]
fn e2e_bpf_generic_maps_behaviour() {
    let mut h: HashMapBpf<u32, String> = HashMapBpf::new(2);
    h.insert(1, "a".into()).unwrap();
    h.insert(2, "b".into()).unwrap();
    assert!(h.insert(3, "c".into()).is_err());

    let mut lru: LruMap<u32, String> = LruMap::new(2);
    lru.insert(1, "a".into());
    lru.insert(2, "b".into());
    let _ = lru.lookup(&1);
    let evicted = lru.insert(3, "c".into());
    assert_eq!(evicted.unwrap().0, 2);

    let mut t: LpmTrie<u32> = LpmTrie::new();
    t.insert("10.0.0.0/8", 1).unwrap();
    t.insert("10.10.5.0/24", 3).unwrap();
    assert_eq!(t.lookup(ip4(10, 10, 5, 7)), Some(3));
    assert_eq!(t.lookup(ip4(10, 99, 0, 1)), Some(1));
    assert_eq!(t.lookup(ip4(11, 0, 0, 1)), None);
}

// ── E2E #36: ExternalWorkload lifecycle ───────────────────────────────────

#[test]
fn e2e_external_workload_lifecycle() {
    let tenant = TenantId::new("e2e-36");
    let mut m = ExternalWorkloadManager::new(tenant.clone(), 30);
    m.register(ExternalWorkloadSpec {
        name: "vm-1".into(), tenant,
        ipv4: Some(ip4(192, 168, 1, 10)), ipv6: None,
        labels: BTreeMap::new(), trust_domain: "cluster.local".into(),
    }).unwrap();
    m.transition("vm-1", WorkloadState::Connected).unwrap();
    m.transition("vm-1", WorkloadState::Ready).unwrap();
    m.heartbeat("vm-1", 0).unwrap();
    assert_eq!(m.ready_count(), 1);
    let stale = m.sweep_stale(40_000_000_000);
    assert_eq!(stale, 1);
    assert_eq!(m.ready_count(), 0);
}

// ── E2E #37: CiliumNode CRD + per-node CIDR allocation ─────────────────────

#[test]
fn e2e_cilium_node_crd_and_cidr_alloc() {
    let tenant = TenantId::new("e2e-37");
    let mut store = CiliumNodeStore::new(tenant.clone());
    store.configure_cluster_pool("10.244.0.0/16", 24).unwrap();
    store.register(CiliumNodeSpec {
        name: "node-a".into(), tenant,
        addresses: vec![NodeAddress { ip: ip4(10, 0, 0, 1), kind: AddressKind::InternalIP }],
        ipam: NodeIpamSpec {
            pod_cidrs: vec![], used_ipv4: 0, used_ipv6: 0,
            pre_allocate: 8, max_above_watermark: 16,
        },
        encryption_key: 0, cluster_id: 1,
    }).unwrap();
    let subnet = store.allocate_pod_cidr("node-a").unwrap();
    assert_eq!(subnet, "10.244.0.0/24");
    let (sp, _st) = store.lookup("node-a").unwrap();
    assert!(sp.ipam.pod_cidrs.contains(&"10.244.0.0/24".to_string()));
}

// ── E2E #38: Status board worst-state aggregation ─────────────────────────

#[test]
fn e2e_status_board_worst_state_wins() {
    let tenant = TenantId::new("e2e-38");
    let mut b = StatusBoard::new(tenant, 30);
    b.report(ComponentName::Kvstore, ComponentStatus::ok("connected"), 100);
    b.report(ComponentName::IPAM, ComponentStatus::degraded("low pool"), 100);
    b.report(ComponentName::Encryption, ComponentStatus::disabled(), 100);
    let agg = b.aggregate(100);
    assert_eq!(agg.overall, DaemonState::Degraded);
    assert!(agg.components.contains_key("kvstore"));
    assert!(agg.components.contains_key("ipam"));
}

// ── E2E #39: Endpoint regen queue coalesces and pipelines ────────────────

#[test]
fn e2e_endpoint_regen_coalesce_and_pipeline() {
    let tenant = TenantId::new("e2e-39");
    let mut c = RegenController::new(tenant);
    c.enqueue(RegenRequest { endpoint_id: 1, level: RegenLevel::Maps, reason: "ip change".into(), enqueued_ns: 0 });
    let merged = c.enqueue(RegenRequest { endpoint_id: 1, level: RegenLevel::PolicyRecompute, reason: "policy".into(), enqueued_ns: 100 });
    assert_eq!(merged.level, RegenLevel::PolicyRecompute);
    let popped = c.pop_for_processing(200).unwrap();
    assert_eq!(popped.endpoint_id, 1);
    c.complete(1, 300).unwrap();
    assert_eq!(c.completed, 1);
}

// ── E2E #40: Recorder pcap-style capture ─────────────────────────────────

#[test]
fn e2e_recorder_5tuple_capture() {
    let tenant = TenantId::new("e2e-40");
    let mut r = Recorder::new(tenant, 100);
    r.upsert_policy(RecorderPolicy {
        id: 1, priority: 100,
        tuple: RecorderTuple {
            src_ip: None, dst_ip: Some(ip4(10, 96, 0, 1)),
            src_port: 0, dst_port: 80, protocol: RecorderProto::Tcp,
        },
        capture_length: 64, sample_one_in_n: 1,
    }).unwrap();
    let captured = r.capture(ip4(10, 0, 0, 1), ip4(10, 96, 0, 1), 1234, 80, 6, 100, &[0xAB; 1500]);
    assert_eq!(captured, Some(1));
    let pkts = r.drain(1).unwrap();
    assert_eq!(pkts.len(), 1);
    assert_eq!(pkts[0].bytes.len(), 64);
}

// ── E2E #41: Maps GC scheduler ───────────────────────────────────────────

#[test]
fn e2e_maps_gc_dispatch() {
    let tenant = TenantId::new("e2e-41");
    let mut c = MapsGcController::new(tenant);
    c.schedule(GcTarget::Conntrack, 60).unwrap();
    c.schedule(GcTarget::Nat, 30).unwrap();
    let due = c.due(0);
    assert_eq!(due.len(), 2);
    c.record(GcSweepReport { target: GcTarget::Conntrack, scanned: 100, deleted: 5, duration_us: 50, timestamp_ns: 0 });
    let due_after = c.due(31_000_000_000);
    assert_eq!(due_after, vec![GcTarget::Nat]);
}

// ── E2E #42: ProxyHealth state escalation ────────────────────────────────

#[test]
fn e2e_proxy_health_failure_threshold() {
    let tenant = TenantId::new("e2e-42");
    let mut c = ProxyHealthChecker::new(tenant, 3);
    c.register("envoy");
    for ts in 0..2u64 {
        c.record("envoy", ProxyProbe { timestamp_ns: ts, success: false, status_code: 500, latency_us: 50 }).unwrap();
    }
    assert_eq!(c.status("envoy").unwrap().state, ProxyState::Degraded);
    c.record("envoy", ProxyProbe { timestamp_ns: 3, success: false, status_code: 500, latency_us: 50 }).unwrap();
    assert_eq!(c.status("envoy").unwrap().state, ProxyState::Down);
    c.record("envoy", ProxyProbe { timestamp_ns: 4, success: true, status_code: 200, latency_us: 10 }).unwrap();
    assert_eq!(c.status("envoy").unwrap().state, ProxyState::Live);
}

// ── E2E #43: Readiness gate flips to ready ───────────────────────────────

#[test]
fn e2e_readiness_gate_flip_to_ready() {
    let tenant = TenantId::new("e2e-43");
    let mut g = ReadinessGateController::new(tenant);
    g.register("default", "p1", 100);
    assert_eq!(g.status("default", "p1").unwrap().status, GateStatus::Pending);
    g.set_ready("default", "p1", 200).unwrap();
    assert_eq!(g.status("default", "p1").unwrap().status, GateStatus::Ready);
}

// ── E2E #44: RemoteCluster lifecycle stale detection ─────────────────────

#[test]
fn e2e_remote_cluster_stale_detection() {
    let mut s = RemoteClusterStatus::new("us-east");
    s.transition(RemoteClusterState::Synced).unwrap();
    s.heartbeat(100);
    s.check_stale(200, 30);
    assert_eq!(s.state, RemoteClusterState::Failed);
}

// ── E2E #45: Connectivity test framework ─────────────────────────────────

#[test]
fn e2e_connectivity_test_suite_report() {
    let tenant = TenantId::new("e2e-45");
    let mut s = ConnectivitySuite::new(tenant, "default-suite");
    s.add(CtTest {
        name: "client→api allowed".into(), kind: TestKind::PodToPod,
        source: "client".into(), destination: "api".into(),
        expected: ExpectedVerdict::Allow, actual: None,
    }).unwrap();
    s.add(CtTest {
        name: "client→world denied".into(), kind: TestKind::PodToWorld,
        source: "client".into(), destination: "8.8.8.8".into(),
        expected: ExpectedVerdict::Deny, actual: None,
    }).unwrap();
    s.record("client→api allowed", ActualOutcome::Allowed { duration_ms: 5 }).unwrap();
    s.record("client→world denied", ActualOutcome::Denied).unwrap();
    let r = s.report();
    assert_eq!(r.total, 2);
    assert_eq!(r.passed, 2);
    assert_eq!(r.failed, 0);
}

// ── E2E #46: Endpoint lifecycle with canonical chain ─────────────────────

#[test]
fn e2e_endpoint_lifecycle_with_canonical_program_chain() {
    let tenant = TenantId::new("e2e-46");
    let mut mgr = EndpointManager::new();
    let id = mgr.create(tenant, "client", "default", ip4(10, 0, 1, 5));
    mgr.transition(id, EndpointState::WaitingForIdentity).unwrap();
    mgr.transition(id, EndpointState::Ready).unwrap();
    mgr.set_program_chain(id, canonical_egress_chain()).unwrap();
    let ep = mgr.lookup(id).unwrap();
    assert_eq!(ep.state, EndpointState::Ready);
    assert_eq!(ep.program_chain.len(), 7);
    assert_eq!(ep.program_chain[0], EndpointBpfProgram::FromContainer);
    assert_eq!(ep.program_chain[6], EndpointBpfProgram::ToLxc);
}

// ── E2E #47: Hubble topology graph builder ──────────────────────────────

#[test]
fn e2e_hubble_topology_graph() {
    let tenant = TenantId::new("e2e-47");
    let flows: Vec<FlowLog> = (0..5).map(|_| FlowLog {
        tenant: tenant.clone(),
        time: chrono::Utc::now(),
        source_identity: 256, destination_identity: 257,
        source_pod: "ns/client".into(), destination_pod: "ns/api".into(),
        verdict: HubbleVerdict::Forwarded,
        drop_reason: DropReason::None,
        bytes: 100,
    }).collect();
    let g = TopologyGraph::build(&tenant, &flows);
    let edge = g.edge(256, 257).unwrap();
    assert_eq!(edge.forwarded, 5);
    assert_eq!(edge.bytes, 500);
    assert_eq!(g.node_count(), 2);
}

// ── E2E #48: Hubble per-node aggregator ─────────────────────────────────

#[test]
fn e2e_hubble_node_aggregator_filters_cross_tenant() {
    let tenant = TenantId::new("e2e-48");
    let mut agg = NodeAggregator::new(tenant.clone());
    agg.ingest("node-a", &FlowLog {
        tenant: tenant.clone(), time: chrono::Utc::now(),
        source_identity: 1, destination_identity: 2,
        source_pod: "ns/a".into(), destination_pod: "ns/b".into(),
        verdict: HubbleVerdict::Forwarded,
        drop_reason: DropReason::None, bytes: 100,
    });
    agg.ingest("node-a", &FlowLog {
        tenant: TenantId::new("other"), time: chrono::Utc::now(),
        source_identity: 1, destination_identity: 2,
        source_pod: "ns/a".into(), destination_pod: "ns/b".into(),
        verdict: HubbleVerdict::Forwarded,
        drop_reason: DropReason::None, bytes: 100,
    });
    let s = agg.summary("node-a").unwrap();
    assert_eq!(s.flows_total, 1);
}

// ── E2E #49: FQDN match-pattern semantics ────────────────────────────────

#[test]
fn e2e_fqdn_match_pattern_subdomain() {
    assert!(match_pattern("*.example.com", "api.example.com"));
    assert!(!match_pattern("*.example.com", "example.com"));
    assert!(!match_pattern("*.example.com", "api.sub.example.com"));
    assert!(match_pattern("*", "single"));
    assert!(!match_pattern("*", "two.parts"));
}

// ── E2E #50: BPF map MapId stable identifiers ────────────────────────────

#[test]
fn e2e_bpfmaps_mapid_taxonomy() {
    use cave_net::cilium::bpfmaps::MapId;
    assert_eq!(MapId::Endpoints.upstream_path(), "bpf/cilium_endpoints.h");
    assert_eq!(MapId::Ipcache.upstream_path(), "bpf/cilium_ipcache.h");
    assert_eq!(MapId::Policy.upstream_path(), "bpf/cilium_policy.h");
    assert_eq!(MapId::CtTcp.upstream_path(), "bpf/cilium_ct_tcp4.h");
}

// ── E2E #51: SocketLB cgroup hook coverage ───────────────────────────────

#[test]
fn e2e_sock_lb_cgroup_hook_names() {
    assert_eq!(CgroupHook::InetConnect4.name(), "BPF_CGROUP_INET4_CONNECT");
    assert_eq!(CgroupHook::UdpRecvmsg4.name(), "BPF_CGROUP_UDP4_RECVMSG");
    assert_eq!(CgroupHook::InetGetpeername4.name(), "BPF_CGROUP_INET4_GETPEERNAME");
}

// ── E2E #52: ServiceRegistry NodePort range guard ────────────────────────

#[test]
fn e2e_service_nodeport_out_of_range_rejected() {
    let tenant = TenantId::new("e2e-52");
    let mut reg = ServiceRegistry::new();
    let mut svc = Service::cluster_ip("svc", "default", tenant, ip4(10, 96, 0, 5), 80);
    svc.service_type = ServiceType::NodePort;
    svc.ports[0].node_port = Some(80);
    let err = reg.upsert(svc).unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("80"));
}

// ── E2E #53: Egress decision + gateway IP ────────────────────────────────

#[test]
fn e2e_egress_decision_has_distinct_egress_ip() {
    let tenant = TenantId::new("e2e-53");
    let mut mgr = EgressManager::new();
    let mut p = EgressGatewayPolicy::new("p", tenant, endpoint_sel(&[("app", "web")]), ip4(192, 0, 2, 1));
    p.destination_cidrs = vec!["1.0.0.0/8".into()];
    p.gateway_nodes = vec![GatewayNode::new("gw", ip4(10, 0, 0, 5))];
    mgr.upsert(p).unwrap();
    let dec = mgr.evaluate(&ls(&[("app", "web")]), &ls(&[]), ip4(1, 1, 1, 1), 0).unwrap().unwrap();
    assert_eq!(dec.egress_ip, ip4(192, 0, 2, 1));
    assert_eq!(dec.gateway_node_ip, ip4(10, 0, 0, 5));
}

// ── E2E #54: SnatTable port allocation idempotency ───────────────────────

#[test]
fn e2e_snat_idempotent_for_same_5tuple() {
    let tenant = TenantId::new("e2e-54");
    let mut t = SnatTable::new(tenant, ip4(192, 168, 1, 1));
    let key = SnatKey { src_ip: ip4(10, 0, 0, 1), src_port: 12345, dst_ip: ip4(8, 8, 8, 8), dst_port: 53 };
    let a = t.allocate(key, 100).unwrap();
    let b = t.allocate(key, 200).unwrap();
    assert_eq!(a.new_src_port, b.new_src_port);
}

// ── E2E #55: Conntrack purge_idle reaps stale UDP entries ────────────────

#[test]
fn e2e_conntrack_purge_idle_udp() {
    let tenant = TenantId::new("e2e-55");
    let mut ct = ConntrackTable::new(tenant, 1024);
    let tup = Tuple::new(ip4(10, 0, 0, 1), 5353, ip4(10, 96, 0, 10), 53, L4Protocol::UDP);
    ct.upsert(tup, CtDirection::Egress, 0, 64, None);
    assert_eq!(ct.len(), 1);
    let purged = ct.purge_idle(60 * 1_000_000_000);
    assert_eq!(purged, 1);
    assert_eq!(ct.len(), 0);
}

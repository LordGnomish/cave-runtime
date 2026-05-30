// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Port of pkg/scheduling/hostportusage_test.go from kubernetes-sigs/karpenter
// v1.12.1 (sha ed490e8). HostPort match semantics (protocol/port/unspecified-IP
// wildcards) plus the per-pod HostPortUsage reservation/conflict tracking and
// the GetHostPorts container-port extraction.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use cave_karpenter::scheduling::hostport::{
    host_ports, ContainerPort, HostPort, HostPortUsage, Protocol,
};

fn hp(ip: IpAddr, port: i32, proto: Protocol) -> HostPort {
    HostPort::new(ip, port, proto)
}
const V4: IpAddr = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 0));

#[test]
fn string_output() {
    let e = hp(V4, 4443, Protocol::Tcp);
    assert_eq!(e.to_string(), "IP=10.0.0.0 Port=4443 Proto=TCP");
}

#[test]
fn identical_entries_match() {
    let e1 = hp(V4, 4443, Protocol::Tcp);
    let e2 = e1.clone();
    assert!(e1.matches(&e2));
    assert!(e2.matches(&e1));
}

#[test]
fn unspecified_ip_matches_either_direction() {
    let e1 = hp(V4, 4443, Protocol::Tcp);
    let mut e2 = hp(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 4443, Protocol::Tcp);
    assert!(e1.matches(&e2));
    assert!(e2.matches(&e1));
    e2 = hp(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 4443, Protocol::Tcp);
    assert!(e1.matches(&e2));
    assert!(e2.matches(&e1));
}

#[test]
fn mismatched_protocols_dont_match() {
    let e1 = hp(V4, 4443, Protocol::Tcp);
    let e2 = hp(V4, 4443, Protocol::Sctp);
    assert!(!e1.matches(&e2));
    assert!(!e2.matches(&e1));
}

#[test]
fn mismatched_ports_dont_match() {
    let e1 = hp(V4, 4443, Protocol::Tcp);
    let e2 = hp(V4, 443, Protocol::Tcp);
    assert!(!e1.matches(&e2));
    assert!(!e2.matches(&e1));
}

#[test]
fn conflicts_across_pods() {
    let mut u = HostPortUsage::new();
    u.add("ns/pod-a", vec![hp(V4, 8080, Protocol::Tcp)]);
    // A different pod claiming the same host port conflicts.
    assert!(u.conflicts("ns/pod-b", &[hp(V4, 8080, Protocol::Tcp)]).is_err());
    // The same pod re-declaring its own port does not conflict with itself.
    assert!(u.conflicts("ns/pod-a", &[hp(V4, 8080, Protocol::Tcp)]).is_ok());
    // A free port does not conflict.
    assert!(u.conflicts("ns/pod-b", &[hp(V4, 9090, Protocol::Tcp)]).is_ok());
    // Unspecified IP on the existing reservation still conflicts a specific IP.
    u.add("ns/pod-c", vec![hp(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 7000, Protocol::Tcp)]);
    assert!(u.conflicts("ns/pod-d", &[hp(V4, 7000, Protocol::Tcp)]).is_err());
}

#[test]
fn delete_pod_frees_ports() {
    let mut u = HostPortUsage::new();
    u.add("ns/pod-a", vec![hp(V4, 8080, Protocol::Tcp)]);
    u.delete_pod("ns/pod-a");
    assert!(u.conflicts("ns/pod-b", &[hp(V4, 8080, Protocol::Tcp)]).is_ok());
}

#[test]
fn get_host_ports_extraction() {
    let ports = vec![
        // host_port 0 is skipped
        ContainerPort { host_ip: String::new(), host_port: 0, protocol: Protocol::Tcp },
        // empty host_ip defaults to 0.0.0.0
        ContainerPort { host_ip: String::new(), host_port: 8080, protocol: Protocol::Tcp },
        ContainerPort { host_ip: "192.168.1.1".into(), host_port: 9090, protocol: Protocol::Udp },
    ];
    let out = host_ports(&ports);
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].to_string(), "IP=0.0.0.0 Port=8080 Proto=TCP");
    assert_eq!(out[1].to_string(), "IP=192.168.1.1 Port=9090 Proto=UDP");
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Userspace **DSR (Direct Server Return)** IPv4-option datapath tests.
//!
//! Cite: cilium/bpf/lib/nodeport.h (v1.19.3) — `dsr_set_opt4`,
//! `dsr_extract_opt4`, `struct dsr_opt_v4`, `DSR_IPV4_OPT_TYPE`.
//!
//! In DSR mode the load balancer forwards a client packet to the chosen
//! backend while preserving the client source IP; the backend replies
//! straight to the client (bypassing the LB). To let the backend node
//! reverse-translate replies, Cilium embeds the original service
//! VIP+port in an 8-byte IPv4 option (`type, len, port, addr`):
//!
//!   * `dsr_set_opt4` writes the option on the LB node — only on the
//!     TCP SYN (later packets ride a NAT entry), growing `ihl` by two
//!     32-bit words and `tot_len` by 8. UDP always carries it.
//!   * `dsr_extract_opt4` reads it back on the backend node when
//!     `ihl >= 7`, recovering `(svc_addr, svc_port)`.
//!
//! This exercises the option encode/decode + header-length bookkeeping.
//! Packet-buffer adjustment and L3 checksum recompute are kernel-owned.

use cave_net::ebpf_sim::dsr_sim::{
    dsr_extract_opt4, dsr_set_opt4, DsrDrop, DsrOptV4, DsrSetOutcome, Ipv4Hdr, DSR_IPV4_OPT_TYPE,
    DSR_OPT_V4_LEN, IPOPT_COPY,
};

const IPPROTO_TCP: u8 = 6;
const IPPROTO_UDP: u8 = 17;

fn svc_addr() -> u32 {
    u32::from_be_bytes([172, 20, 0, 1])
}

#[test]
fn opt_type_is_copy_flag_or_0x1a() {
    // IPOPT_COPY | 26 == 0x9a; copied to each fragment.
    assert_eq!(IPOPT_COPY, 0x80);
    assert_eq!(DSR_IPV4_OPT_TYPE, 0x9a);
    assert_eq!(DSR_OPT_V4_LEN, 8);
}

#[test]
fn option_wire_layout_is_type_len_port_addr() {
    let opt = DsrOptV4::for_service(svc_addr(), 443);
    let b = opt.to_bytes();
    assert_eq!(b[0], DSR_IPV4_OPT_TYPE);
    assert_eq!(b[1], 8); // len
    assert_eq!(&b[2..4], &443u16.to_be_bytes()); // port, network order
    assert_eq!(&b[4..8], &svc_addr().to_be_bytes()); // addr, network order
    // Round-trips through from_bytes.
    let back = DsrOptV4::from_bytes(&b);
    assert_eq!(back.addr, svc_addr());
    assert_eq!(back.port, 443);
    assert_eq!(back.type_, DSR_IPV4_OPT_TYPE);
    assert_eq!(back.len, 8);
}

#[test]
fn set_opt4_on_tcp_syn_grows_header_and_encodes_service() {
    let mut ip4 = Ipv4Hdr { ihl: 5, tot_len: 40, protocol: IPPROTO_TCP };
    let out = dsr_set_opt4(&mut ip4, svc_addr(), 8080, /*tcp_syn=*/ true, None);
    match out {
        DsrSetOutcome::Set { opt } => {
            assert_eq!(opt.addr, svc_addr());
            assert_eq!(opt.port, 8080);
        }
        other => panic!("expected Set, got {other:?}"),
    }
    // ihl += 8>>2 == 2 words; tot_len += sizeof(opt) == 8.
    assert_eq!(ip4.ihl, 7);
    assert_eq!(ip4.tot_len, 48);
}

#[test]
fn set_opt4_skips_non_syn_tcp() {
    let mut ip4 = Ipv4Hdr { ihl: 5, tot_len: 40, protocol: IPPROTO_TCP };
    let out = dsr_set_opt4(&mut ip4, svc_addr(), 8080, /*tcp_syn=*/ false, None);
    assert!(matches!(out, DsrSetOutcome::SkipNonSyn));
    // No mutation on the non-SYN path.
    assert_eq!(ip4.ihl, 5);
    assert_eq!(ip4.tot_len, 40);
}

#[test]
fn set_opt4_on_udp_always_encodes() {
    let mut ip4 = Ipv4Hdr { ihl: 5, tot_len: 32, protocol: IPPROTO_UDP };
    // tcp_syn flag is irrelevant for UDP.
    let out = dsr_set_opt4(&mut ip4, svc_addr(), 53, false, None);
    assert!(matches!(out, DsrSetOutcome::Set { .. }));
    assert_eq!(ip4.ihl, 7);
    assert_eq!(ip4.tot_len, 40);
}

#[test]
fn set_opt4_drops_when_options_would_overflow() {
    // ipv4_hdrlen(ihl=14)=56; 56 + 8 = 64 > 20 + MAX_IPOPTLEN(40) = 60.
    let mut ip4 = Ipv4Hdr { ihl: 14, tot_len: 60, protocol: IPPROTO_UDP };
    let out = dsr_set_opt4(&mut ip4, svc_addr(), 53, false, None);
    assert!(matches!(out, DsrSetOutcome::Drop(DsrDrop::CtInvalidHdr)));
    assert_eq!(ip4.ihl, 14, "drop must not mutate the header");
}

#[test]
fn set_opt4_frag_needed_when_over_mtu() {
    // tot_len + 8 exceeds the MTU => DROP_FRAG_NEEDED (dsr_is_too_big).
    let mut ip4 = Ipv4Hdr { ihl: 5, tot_len: 1496, protocol: IPPROTO_UDP };
    let out = dsr_set_opt4(&mut ip4, svc_addr(), 53, false, Some(1500));
    assert!(matches!(out, DsrSetOutcome::Drop(DsrDrop::FragNeeded)));
    assert_eq!(ip4.tot_len, 1496);
}

#[test]
fn extract_opt4_recovers_service_after_set() {
    let mut ip4 = Ipv4Hdr { ihl: 5, tot_len: 40, protocol: IPPROTO_TCP };
    let opt = match dsr_set_opt4(&mut ip4, svc_addr(), 8080, true, None) {
        DsrSetOutcome::Set { opt } => opt,
        _ => panic!("set failed"),
    };
    // Egress/backend node reads the option back out.
    let got = dsr_extract_opt4(&ip4, &opt.to_bytes());
    assert_eq!(got, Some((svc_addr(), 8080)));
}

#[test]
fn extract_opt4_none_when_no_option_present() {
    // ihl == 5 => no option words, so nothing to extract.
    let ip4 = Ipv4Hdr { ihl: 5, tot_len: 40, protocol: IPPROTO_TCP };
    let opt = DsrOptV4::for_service(svc_addr(), 8080);
    assert_eq!(dsr_extract_opt4(&ip4, &opt.to_bytes()), None);
}

#[test]
fn extract_opt4_none_when_option_type_mismatch() {
    let ip4 = Ipv4Hdr { ihl: 7, tot_len: 48, protocol: IPPROTO_TCP };
    // A non-DSR IPv4 option sitting in the same byte range.
    let mut bytes = DsrOptV4::for_service(svc_addr(), 8080).to_bytes();
    bytes[0] = 0x83; // LSRR, not the DSR option type
    assert_eq!(dsr_extract_opt4(&ip4, &bytes), None);
}

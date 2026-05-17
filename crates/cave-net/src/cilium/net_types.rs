// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Low-level network address types — IPv4/IPv6/MAC/PortMap/XFRM.
//!
//! Mirrors `pkg/types/`. Upstream defines small thin wrappers around
//! 4/16/6-byte arrays plus the few invariants the agent needs (zero
//! detection, equality, string formatting). We follow the same shape.

use crate::cilium::types::Cite;
use serde::{Deserialize, Serialize};
use std::fmt;

/// 4-byte IPv4 address. Mirrors `pkg/types/ipv4.go::IPv4`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct IPv4(pub [u8; 4]);

impl IPv4 {
    pub const fn new(b: [u8; 4]) -> Self { IPv4(b) }
    pub fn is_zero(self) -> bool { self.0 == [0; 4] }
    pub fn as_u32_be(self) -> u32 { u32::from_be_bytes(self.0) }
}

impl fmt::Display for IPv4 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}.{}", self.0[0], self.0[1], self.0[2], self.0[3])
    }
}

/// 16-byte IPv6 address. Mirrors `pkg/types/ipv6.go::IPv6`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct IPv6(pub [u8; 16]);

impl IPv6 {
    pub const fn new(b: [u8; 16]) -> Self { IPv6(b) }
    pub fn is_zero(self) -> bool { self.0 == [0; 16] }
}

impl fmt::Display for IPv6 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Compressed format. Find the longest run of zero pairs.
        let mut groups = [0u16; 8];
        for i in 0..8 {
            groups[i] = ((self.0[2 * i] as u16) << 8) | self.0[2 * i + 1] as u16;
        }
        // Find longest run of zero groups
        let (mut best_start, mut best_len) = (None::<usize>, 0usize);
        let mut cur_start: Option<usize> = None;
        let mut cur_len = 0;
        for (i, g) in groups.iter().enumerate() {
            if *g == 0 {
                if cur_start.is_none() { cur_start = Some(i); cur_len = 0; }
                cur_len += 1;
                if cur_len > best_len { best_len = cur_len; best_start = cur_start; }
            } else {
                cur_start = None; cur_len = 0;
            }
        }
        let best_end = best_start.map(|s| s + best_len);
        let mut out = String::new();
        let mut i = 0;
        while i < 8 {
            if Some(i) == best_start && best_len > 1 {
                out.push_str("::");
                i = best_end.unwrap();
                continue;
            }
            if i > 0 && !out.ends_with(':') { out.push(':'); }
            out.push_str(&format!("{:x}", groups[i]));
            i += 1;
        }
        if out.is_empty() { out.push_str("::"); }
        f.write_str(&out)
    }
}

/// MAC address (6 bytes). Mirrors `pkg/types/macaddr.go::MACAddr`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MACAddr(pub [u8; 6]);

impl MACAddr {
    pub const fn new(b: [u8; 6]) -> Self { MACAddr(b) }
    pub fn is_zero(self) -> bool { self.0 == [0; 6] }
}

impl fmt::Display for MACAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            self.0[0], self.0[1], self.0[2], self.0[3], self.0[4], self.0[5])
    }
}

/// Single forward port mapping (host:proto:container). Mirrors
/// `pkg/types/portmap.go::PortMap` shape used by the CNI plugin chain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortMap {
    pub host_port: u16,
    pub container_port: u16,
    pub protocol: String,
}

/// XFRM policy direction (in/out/fwd). Mirrors `pkg/types/xfrm.go::Dir`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum XfrmDir { In, Out, Fwd }

impl XfrmDir {
    pub fn as_str(self) -> &'static str {
        match self { XfrmDir::In => "in", XfrmDir::Out => "out", XfrmDir::Fwd => "fwd" }
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/types/ipv4.go", "IPv4");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;

    #[test]
    fn ipv4_displays_dotted_quad() {
        let (_c, _t) = cilium_test_ctx!("pkg/types/ipv4.go", "Display", "tenant-typ-v4d");
        assert_eq!(format!("{}", IPv4::new([10, 0, 0, 1])), "10.0.0.1");
    }

    #[test]
    fn ipv4_zero_is_detected() {
        let (_c, _t) = cilium_test_ctx!("pkg/types/ipv4.go", "Zero", "tenant-typ-v4z");
        assert!(IPv4::new([0; 4]).is_zero());
        assert!(!IPv4::new([1, 0, 0, 0]).is_zero());
    }

    #[test]
    fn ipv4_be_u32_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/types/ipv4.go", "U32", "tenant-typ-v4u");
        let ip = IPv4::new([192, 168, 1, 1]);
        assert_eq!(ip.as_u32_be(), 0xc0a80101);
    }

    #[test]
    fn ipv6_zero_compresses_to_double_colon() {
        let (_c, _t) = cilium_test_ctx!("pkg/types/ipv6.go", "ZeroDisplay", "tenant-typ-v6z");
        assert_eq!(format!("{}", IPv6::new([0; 16])), "::");
    }

    #[test]
    fn ipv6_loopback_compresses() {
        let (_c, _t) = cilium_test_ctx!("pkg/types/ipv6.go", "Loopback", "tenant-typ-v6l");
        let mut b = [0u8; 16]; b[15] = 1;
        assert_eq!(format!("{}", IPv6::new(b)), "::1");
    }

    #[test]
    fn ipv6_simple_addr_display() {
        let (_c, _t) = cilium_test_ctx!("pkg/types/ipv6.go", "Simple", "tenant-typ-v6s");
        // 2001:db8::1
        let mut b = [0u8; 16];
        b[0] = 0x20; b[1] = 0x01;
        b[2] = 0x0d; b[3] = 0xb8;
        b[15] = 1;
        assert_eq!(format!("{}", IPv6::new(b)), "2001:db8::1");
    }

    #[test]
    fn ipv6_full_no_compression() {
        let (_c, _t) = cilium_test_ctx!("pkg/types/ipv6.go", "Full", "tenant-typ-v6f");
        let mut b = [0u8; 16];
        for i in 0..16 { b[i] = (i + 1) as u8; }
        let s = format!("{}", IPv6::new(b));
        // Each group is non-zero.
        assert!(!s.contains("::"));
        assert!(s.contains("102:304"));
    }

    #[test]
    fn macaddr_lowercase_hex_with_colons() {
        let (_c, _t) = cilium_test_ctx!("pkg/types/macaddr.go", "Display", "tenant-typ-mac");
        let m = MACAddr::new([0x02, 0x42, 0xac, 0x11, 0x00, 0x02]);
        assert_eq!(format!("{}", m), "02:42:ac:11:00:02");
    }

    #[test]
    fn macaddr_zero_is_detected() {
        let (_c, _t) = cilium_test_ctx!("pkg/types/macaddr.go", "Zero", "tenant-typ-mz");
        assert!(MACAddr::new([0; 6]).is_zero());
    }

    #[test]
    fn portmap_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/types/portmap.go", "Serde", "tenant-typ-pm");
        let p = PortMap { host_port: 8080, container_port: 80, protocol: "tcp".into() };
        let j = serde_json::to_string(&p).unwrap();
        let back: PortMap = serde_json::from_str(&j).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn xfrm_dir_strings() {
        let (_c, _t) = cilium_test_ctx!("pkg/types/xfrm.go", "Dir", "tenant-typ-xf");
        assert_eq!(XfrmDir::In.as_str(), "in");
        assert_eq!(XfrmDir::Out.as_str(), "out");
        assert_eq!(XfrmDir::Fwd.as_str(), "fwd");
    }

    #[test]
    fn ipv4_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/types/ipv4.go", "Serde", "tenant-typ-v4se");
        let ip = IPv4::new([10, 1, 2, 3]);
        let j = serde_json::to_string(&ip).unwrap();
        let back: IPv4 = serde_json::from_str(&j).unwrap();
        assert_eq!(ip, back);
    }

    #[test]
    fn ipv6_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/types/ipv6.go", "Serde", "tenant-typ-v6se");
        let ip = IPv6::new([0x20,0x01,0x0d,0xb8,0,0,0,0,0,0,0,0,0,0,0,1]);
        let j = serde_json::to_string(&ip).unwrap();
        let back: IPv6 = serde_json::from_str(&j).unwrap();
        assert_eq!(ip, back);
    }
}

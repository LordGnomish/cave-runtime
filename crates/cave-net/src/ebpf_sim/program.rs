// SPDX-License-Identifier: AGPL-3.0-or-later
//! `Program` trait + `Context` + `Verdict` shared shape.
//!
//! A BPF program in Cilium is a function `int prog(struct __sk_buff *)`
//! returning one of `TC_ACT_OK` / `TC_ACT_SHOT` / `TC_ACT_REDIRECT`.
//! Our simulator uses a typed `Verdict` enum and a `Context` carrying
//! the (small) per-packet metadata the simulated programs read.

use crate::ebpf_sim::helpers::Helpers;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    /// `TC_ACT_OK` — keep going through the stack.
    Pass,
    /// `TC_ACT_SHOT` — drop the packet.
    Drop,
    /// `TC_ACT_REDIRECT` — send to `ifindex`.
    Redirect { ifindex: u32 },
}

/// L3/L4 metadata the simulator passes into each program. NOT a
/// real packet buffer — Cilium tests under the kernel exercise full
/// header parsing; we only model the fields control-plane lookups
/// touch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Context {
    pub src_ip: Ipv4,
    pub dst_ip: Ipv4,
    pub src_port: u16,
    pub dst_port: u16,
    pub proto: L4Proto,
    /// ifindex (incoming interface). 0 if unknown.
    pub ifindex: u32,
    /// Cilium "security identity" of the source endpoint. 0 for
    /// "unspecified" / world.
    pub src_identity: u32,
    /// Cilium "security identity" of the destination.
    pub dst_identity: u32,
}

impl Context {
    pub fn new(src: Ipv4, dst: Ipv4, src_port: u16, dst_port: u16, proto: L4Proto) -> Self {
        Self {
            src_ip: src,
            dst_ip: dst,
            src_port,
            dst_port,
            proto,
            ifindex: 0,
            src_identity: 0,
            dst_identity: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Ipv4(pub u32);

impl Ipv4 {
    pub fn from_octets(a: u8, b: u8, c: u8, d: u8) -> Self {
        Self(u32::from_be_bytes([a, b, c, d]))
    }

    pub fn octets(&self) -> [u8; 4] {
        self.0.to_be_bytes()
    }
}

impl std::fmt::Display for Ipv4 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let o = self.octets();
        write!(f, "{}.{}.{}.{}", o[0], o[1], o[2], o[3])
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum L4Proto {
    Tcp,
    Udp,
    Icmp,
    Sctp,
}

impl L4Proto {
    pub fn proto_num(&self) -> u8 {
        match self {
            L4Proto::Tcp => 6,
            L4Proto::Udp => 17,
            L4Proto::Icmp => 1,
            L4Proto::Sctp => 132,
        }
    }
}

pub trait Program: Send + Sync {
    fn name(&self) -> &'static str;
    fn run(&mut self, ctx: &Context, helpers: &Helpers) -> Verdict;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipv4_round_trips_through_octets() {
        let ip = Ipv4::from_octets(10, 0, 0, 1);
        assert_eq!(ip.octets(), [10, 0, 0, 1]);
        assert_eq!(ip.to_string(), "10.0.0.1");
    }

    #[test]
    fn l4_proto_numbers_match_iana() {
        assert_eq!(L4Proto::Tcp.proto_num(), 6);
        assert_eq!(L4Proto::Udp.proto_num(), 17);
        assert_eq!(L4Proto::Icmp.proto_num(), 1);
        assert_eq!(L4Proto::Sctp.proto_num(), 132);
    }

    #[test]
    fn context_defaults_zero_identities_and_ifindex() {
        let c = Context::new(
            Ipv4::from_octets(10, 0, 0, 1),
            Ipv4::from_octets(10, 0, 0, 2),
            12345,
            80,
            L4Proto::Tcp,
        );
        assert_eq!(c.ifindex, 0);
        assert_eq!(c.src_identity, 0);
        assert_eq!(c.dst_identity, 0);
    }
}

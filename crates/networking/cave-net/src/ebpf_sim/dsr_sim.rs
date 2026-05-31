// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Userspace simulation of Cilium's IPv4 **DSR (Direct Server Return)**
//! option datapath.
//!
//! Cite: cilium/bpf/lib/nodeport.h (v1.19.3) — `dsr_set_opt4`,
//!       `dsr_extract_opt4`, `struct dsr_opt_v4`, `DSR_IPV4_OPT_TYPE`.
//!
//! In DSR mode the load balancer node DNATs a client packet to the
//! selected backend **without** SNATing the source, so the backend
//! replies directly to the client and the reply never transits the LB.
//! For the backend node to reverse-translate those replies it must know
//! the original service VIP+port — Cilium carries them in an 8-byte
//! IPv4 option appended after the base header:
//!
//! ```text
//!   struct dsr_opt_v4 {        // 2 × 32-bit words
//!       __u8  type;            // DSR_IPV4_OPT_TYPE (IPOPT_COPY | 0x1a)
//!       __u8  len;             // 8
//!       __u16 port;            // svc_port,  network byte order
//!       __u32 addr;            // svc_addr,  network byte order
//!   };
//! ```
//!
//! `dsr_set_opt4` writes it on the LB node — for TCP only on the SYN
//! (later packets of the flow ride a conntrack/NAT entry on the backend
//! node), for UDP on every packet — bumping `ihl` by two 32-bit words
//! and `tot_len` by 8. `dsr_extract_opt4` reads it back on the backend
//! node whenever `ihl >= 7`.
//!
//! Out of scope (kernel BPF harness owns these): the actual packet
//! buffer head-room adjustment (`ctx_adjust_hroom`), the L3 checksum
//! recompute (`csum_diff` / `ipv4_csum_update_by_diff`), and the Geneve
//! / IP-in-IP DSR encapsulation variants. This sim covers the option
//! encode/decode + the IPv4 header-length bookkeeping, which is the
//! observable behaviour the reverse path depends on.

/// `IPOPT_COPY` — the high bit marking an option that is copied into
/// every fragment of a fragmented datagram.
pub const IPOPT_COPY: u8 = 0x80;

/// `DSR_IPV4_OPT_TYPE` — `IPOPT_COPY | 0x1a` (== `0x9a`). Option number
/// 26, control class, copied to each fragment.
pub const DSR_IPV4_OPT_TYPE: u8 = IPOPT_COPY | 0x1a;

/// `sizeof(struct dsr_opt_v4)` — the option's `len` byte and the number
/// of bytes it occupies (two 32-bit words).
pub const DSR_OPT_V4_LEN: u8 = 8;

/// `MAX_IPOPTLEN` — the IPv4 spec caps options at 40 bytes, so the full
/// header (base 20 + options) may not exceed 60 bytes.
const MAX_IPOPTLEN: u16 = 40;
/// `sizeof(struct iphdr)` — the option-less IPv4 base header.
const IPHDR_LEN: u16 = 20;

/// `struct dsr_opt_v4` — the service identity carried for DSR.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DsrOptV4 {
    pub type_: u8,
    pub len: u8,
    /// Service port in host byte order (the wire form is big-endian).
    pub port: u16,
    /// Service address in host byte order (the wire form is big-endian).
    pub addr: u32,
}

impl DsrOptV4 {
    /// Build the option for a service `(addr, port)` — `type`/`len` are
    /// fixed (`dsr_set_opt4` assigns `opt.type`, `opt.len`).
    pub fn for_service(addr: u32, port: u16) -> Self {
        Self { type_: DSR_IPV4_OPT_TYPE, len: DSR_OPT_V4_LEN, port, addr }
    }

    /// Serialize to the 8 wire bytes: `type, len, port(be16), addr(be32)`.
    /// Port and address are written network byte order, matching the
    /// `bpf_htons(svc_port)` / `bpf_htonl(svc_addr)` stores upstream.
    pub fn to_bytes(&self) -> [u8; 8] {
        let mut b = [0u8; 8];
        b[0] = self.type_;
        b[1] = self.len;
        b[2..4].copy_from_slice(&self.port.to_be_bytes());
        b[4..8].copy_from_slice(&self.addr.to_be_bytes());
        b
    }

    /// Parse the 8 wire bytes back into host byte order.
    pub fn from_bytes(b: &[u8; 8]) -> Self {
        Self {
            type_: b[0],
            len: b[1],
            port: u16::from_be_bytes([b[2], b[3]]),
            addr: u32::from_be_bytes([b[4], b[5], b[6], b[7]]),
        }
    }
}

/// The IPv4 header fields the DSR path reads and mutates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ipv4Hdr {
    /// Internet header length in 32-bit words (5 = no options).
    pub ihl: u8,
    /// Total length (header + payload) in bytes, host order.
    pub tot_len: u16,
    /// L4 protocol number (`IPPROTO_TCP` 6, `IPPROTO_UDP` 17, …).
    pub protocol: u8,
}

impl Ipv4Hdr {
    /// `ipv4_hdrlen` — header length in bytes (`ihl * 4`).
    pub fn hdrlen(&self) -> u16 {
        self.ihl as u16 * 4
    }
}

/// `DROP_*` reasons `dsr_set_opt4` can return.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DsrDrop {
    /// `DROP_CT_INVALID_HDR` — adding the option would overflow the
    /// 60-byte IPv4 header maximum.
    CtInvalidHdr,
    /// `DROP_FRAG_NEEDED` — the grown packet would exceed the egress
    /// MTU (`dsr_is_too_big`); the caller replies with ICMP frag-needed.
    FragNeeded,
}

/// Outcome of `dsr_set_opt4`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DsrSetOutcome {
    /// Option written; the header was grown in place. `opt` is the
    /// encoded service identity.
    Set { opt: DsrOptV4 },
    /// TCP, but not the SYN — the option is set only on the first
    /// packet, so this returns without mutating anything.
    SkipNonSyn,
    /// The option could not be added.
    Drop(DsrDrop),
}

/// `dsr_set_opt4` — embed `(svc_addr, svc_port)` in an IPv4 option on
/// the LB node.
///
/// * TCP: only the SYN carries the option (`tcp_syn == true`); other
///   packets return [`DsrSetOutcome::SkipNonSyn`] untouched.
/// * UDP (and any non-TCP): always set.
///
/// On success `ip4.ihl` grows by `sizeof(opt) >> 2 == 2` words and
/// `ip4.tot_len` by `sizeof(opt) == 8`. `mtu` (when `Some`) triggers
/// [`DsrDrop::FragNeeded`] if the grown packet would not fit.
pub fn dsr_set_opt4(
    ip4: &mut Ipv4Hdr,
    svc_addr: u32,
    svc_port: u16,
    tcp_syn: bool,
    mtu: Option<u16>,
) -> DsrSetOutcome {
    const IPPROTO_TCP: u8 = 6;

    // TCP: set the option only on the SYN.
    if ip4.protocol == IPPROTO_TCP && !tcp_syn {
        return DsrSetOutcome::SkipNonSyn;
    }

    let opt_len = DSR_OPT_V4_LEN as u16;

    // Header must stay within the 60-byte (base + MAX_IPOPTLEN) limit.
    if ip4.hdrlen() + opt_len > IPHDR_LEN + MAX_IPOPTLEN {
        return DsrSetOutcome::Drop(DsrDrop::CtInvalidHdr);
    }

    let tot_len = ip4.tot_len + opt_len;

    // dsr_is_too_big: the grown packet must still fit the egress MTU.
    if let Some(mtu) = mtu {
        if tot_len > mtu {
            return DsrSetOutcome::Drop(DsrDrop::FragNeeded);
        }
    }

    // Mutate the header: +2 words of options, +8 bytes total length.
    ip4.ihl += (opt_len >> 2) as u8;
    ip4.tot_len = tot_len;

    DsrSetOutcome::Set { opt: DsrOptV4::for_service(svc_addr, svc_port) }
}

/// `dsr_extract_opt4` — recover `(svc_addr, svc_port)` on the backend
/// node. Returns `None` unless the header carries options (`ihl >= 7`,
/// i.e. base 5 words + the 2-word DSR option) and the option at word 5
/// is a well-formed DSR option (`type == DSR_IPV4_OPT_TYPE && len == 8`).
pub fn dsr_extract_opt4(ip4: &Ipv4Hdr, opt_bytes: &[u8; 8]) -> Option<(u32, u16)> {
    if ip4.ihl < 0x7 {
        return None;
    }
    let opt = DsrOptV4::from_bytes(opt_bytes);
    if opt.type_ == DSR_IPV4_OPT_TYPE && opt.len == DSR_OPT_V4_LEN {
        Some((opt.addr, opt.port))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_constant_matches_upstream() {
        assert_eq!(DSR_IPV4_OPT_TYPE, 0x9a);
    }

    #[test]
    fn set_then_extract_round_trips() {
        let mut ip4 = Ipv4Hdr { ihl: 5, tot_len: 40, protocol: 17 };
        let opt = match dsr_set_opt4(&mut ip4, 0xC0A80001, 80, false, None) {
            DsrSetOutcome::Set { opt } => opt,
            _ => panic!("set"),
        };
        assert_eq!(dsr_extract_opt4(&ip4, &opt.to_bytes()), Some((0xC0A80001, 80)));
    }
}

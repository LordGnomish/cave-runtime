// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Stateless NAT46/64 address embedding.
//!
//! Cite: cilium/bpf/lib/nat_46x64.h `build_v4_in_v6`,
//!       `build_v4_in_v6_rfc6052`, `get_v4_from_v6`, `is_v4_in_v6`,
//!       `is_v4_in_v6_rfc6052` (pinned v1.19.3, Apache-2.0);
//!       RFC 4291 §2.5.5.2 (IPv4-mapped) + RFC 6052 §2.1 (the well-known
//!       prefix `64:ff9b::/96`).
//!
//! Cilium's NAT46x64 gateway translates between IPv4 and IPv6 by
//! embedding the 32-bit IPv4 address in the low word of an IPv6 address.
//! Two encodings share that low-word layout:
//!
//!   * **IPv4-mapped** `::ffff:a.b.c.d` — bytes `[0..10] = 0`,
//!     `[10] = [11] = 0xff`, `[12..16] = v4`. Cilium uses this as the
//!     internal sentinel for "this v6 packet is really v4" before it
//!     hands the flow back to the v4 datapath.
//!   * **RFC 6052 well-known prefix** `64:ff9b::/96` — bytes
//!     `[0..4] = 00 64 ff 9b`, `[4..12] = 0`, `[12..16] = v4`. This is
//!     the on-the-wire form a NAT64 client sends.
//!
//! `get_v4_from_v6` accepts either encoding and returns the low 32 bits;
//! an address in neither form is rejected (`DROP_INVALID` upstream → we
//! return `None`). The IPv4 octets are preserved verbatim — bytes
//! `[12..16]` map directly to the dotted-quad, never byte-swapped.
//!
//! Out of scope (control-plane / kernel BPF harness): the
//! operator-configurable non-well-known NAT64 prefix, and the L3/L4
//! header rewrite + checksum fixup that the wire translation performs.
//! This sim covers the **address-format embedding and validation**.

use crate::ebpf_sim::program::Ipv4;

/// The RFC 6052 §2.1 well-known prefix `64:ff9b::/96`, high 4 bytes.
pub const RFC6052_WELL_KNOWN_PREFIX: [u8; 4] = [0x00, 0x64, 0xff, 0x9b];

/// A 128-bit IPv6 address as raw network-order octets — the shape of
/// upstream `union v6addr`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct V6Addr(pub [u8; 16]);

impl V6Addr {
    /// The IPv4 carried in the low 32 bits (`[12..16]`), regardless of
    /// which prefix occupies the high bytes.
    fn low_v4(&self) -> Ipv4 {
        Ipv4::from_octets(self.0[12], self.0[13], self.0[14], self.0[15])
    }
}

/// Build the IPv4-mapped form `::ffff:a.b.c.d`. Mirrors `build_v4_in_v6`.
pub fn build_v4_in_v6(v4: Ipv4) -> V6Addr {
    let mut a = [0u8; 16];
    a[10] = 0xff;
    a[11] = 0xff;
    a[12..16].copy_from_slice(&v4.octets());
    V6Addr(a)
}

/// Build the RFC 6052 well-known-prefix form `64:ff9b::a.b.c.d`.
/// Mirrors `build_v4_in_v6_rfc6052`.
pub fn build_v4_in_v6_rfc6052(v4: Ipv4) -> V6Addr {
    let mut a = [0u8; 16];
    a[0..4].copy_from_slice(&RFC6052_WELL_KNOWN_PREFIX);
    a[12..16].copy_from_slice(&v4.octets());
    V6Addr(a)
}

/// True iff `addr` is the IPv4-mapped sentinel `::ffff:0:0/96`
/// (bytes `[0..10] = 0`, `[10] = [11] = 0xff`). Mirrors `is_v4_in_v6`.
pub fn is_v4_in_v6(addr: &V6Addr) -> bool {
    addr.0[0..10].iter().all(|&b| b == 0) && addr.0[10] == 0xff && addr.0[11] == 0xff
}

/// True iff `addr` carries the RFC 6052 well-known prefix `64:ff9b::/96`
/// (bytes `[0..4] = 00 64 ff 9b`, `[4..12] = 0`). Mirrors
/// `is_v4_in_v6_rfc6052`.
pub fn is_v4_in_v6_rfc6052(addr: &V6Addr) -> bool {
    addr.0[0..4] == RFC6052_WELL_KNOWN_PREFIX && addr.0[4..12].iter().all(|&b| b == 0)
}

/// Recover the embedded IPv4 from either encoding, or `None`
/// (`DROP_INVALID`) if `addr` is in neither form. Mirrors
/// `get_v4_from_v6`.
pub fn get_v4_from_v6(addr: &V6Addr) -> Option<Ipv4> {
    if is_v4_in_v6(addr) || is_v4_in_v6_rfc6052(addr) {
        Some(addr.low_v4())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mapped_and_rfc6052_differ_only_in_the_high_prefix() {
        let ip = Ipv4::from_octets(198, 51, 100, 7);
        let mapped = build_v4_in_v6(ip);
        let wkp = build_v4_in_v6_rfc6052(ip);
        // Same low 32 bits...
        assert_eq!(mapped.0[12..16], wkp.0[12..16]);
        // ...different high prefix.
        assert_ne!(mapped.0[0..12], wkp.0[0..12]);
    }

    #[test]
    fn zero_address_is_not_embedded() {
        // All-zero (the unspecified address) matches neither prefix
        // (mapped needs 0xff 0xff; rfc6052 needs the 64:ff9b prefix).
        assert_eq!(get_v4_from_v6(&V6Addr([0u8; 16])), None);
    }
}

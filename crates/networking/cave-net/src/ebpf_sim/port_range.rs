// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Policy port-range → masked-port decomposition.
//!
//! Cite: cilium/pkg/policy/portrange.go `PortRangeToMaskedPorts`
//! (pinned v1.19.3, Apache-2.0).
//!
//! Cilium's datapath indexes L4 policy in a longest-prefix-match trie
//! keyed by `(identity, traffic_direction, nexthdr, dport)`. A port
//! *range* (a NetworkPolicy/CiliumNetworkPolicy `EndPort`) cannot be a
//! single trie key, so the agent decomposes `[start, end]` into the
//! minimal set of `(port, mask)` prefixes that exactly tile the range
//! — the classic "range → CIDR prefixes" algorithm over the 16-bit
//! port space. A wildcard bit in `mask` (0) means "any value here".
//!
//! The decomposition approaches a "middle point" (the highest bit that
//! differs between start and end) from both ends: from `start` it
//! wildcards trailing zeroes then walks up clearing-to-setting each
//! 0-bit; from `end` it wildcards trailing ones then walks up
//! setting-to-clearing each 1-bit. A whole-power-of-two range collapses
//! to a single prefix.

use serde::{Deserialize, Serialize};

/// A port with a wildcard `mask`. The port range is represented as a
/// masked port because the datapath indexes policy keys in a bitwise
/// longest-prefix-match trie. A 0-bit in `mask` wildcards the
/// corresponding `port` bit. Mirrors upstream `policy.MaskedPort`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MaskedPort {
    pub port: u16,
    pub mask: u16,
}

impl MaskedPort {
    /// Number of concrete ports this masked port covers.
    pub fn covered(&self) -> u32 {
        (!self.mask) as u32 + 1
    }

    /// True if `port` falls under this masked prefix.
    pub fn matches(&self, port: u16) -> bool {
        port & self.mask == self.port & self.mask
    }
}

impl std::fmt::Display for MaskedPort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{{port: 0x{:x}, mask: 0x{:x}}}", self.port, self.mask)
    }
}

/// Returns a new `MaskedPort` where the `wildcard_bits` lowest bits are
/// wildcarded. Mirrors upstream `maskedPort`.
fn masked_port(port: u16, wildcard_bits: u32) -> MaskedPort {
    // `<< 16` would overflow a u16; a full-wildcard mask is 0.
    let mask: u16 = if wildcard_bits >= 16 {
        0
    } else {
        u16::MAX << wildcard_bits
    };
    MaskedPort {
        port: port & mask,
        mask,
    }
}

/// Decompose `[start, end]` into the minimal set of masked ports that
/// tile it. Ports are not returned in any particular order — sort by
/// `port` for deterministic comparison. Mirrors upstream
/// `PortRangeToMaskedPorts` verbatim.
///
/// Edge cases (upstream semantics):
///   * `start == 0 && (end == 0 || end == 65535)` → full wildcard `{0, 0}`.
///   * `end <= start` → just the start port, fully masked (`end == 0`
///     means "no range"; other `start >= end` cases are ambiguous and
///     upstream returns the start port).
pub fn port_range_to_masked_ports(start: u16, end: u16) -> Vec<MaskedPort> {
    // This is a wildcard.
    if start == 0 && (end == 0 || end == u16::MAX) {
        return vec![MaskedPort { port: 0, mask: 0 }];
    }
    // This is a single port (also covers the ambiguous start > end cases).
    if end <= start {
        return vec![MaskedPort {
            port: start,
            mask: 0xffff,
        }];
    }

    // Find the number of common leading bits. The first uncommon bit
    // will be 0 for the start and 1 for the end.
    let common_bits = (start ^ end).leading_zeros();

    // Cover the case where all bits after the common bits are zeros on
    // start and ones on end — then the range is a single masked port.
    // E.g. 16-31 (0b10000-0b11111) → 0b1xxxx, not 0b10xxx + 0b11xxx.
    // Also covers the trivial start == end.
    let mask: u16 = u16::MAX >> common_bits;
    if start & mask == 0 && !end & mask == 0 {
        return vec![masked_port(start, 16 - common_bits)];
    }

    // The "middle point" toward which both sides approach: the highest
    // bit that differs between start and end.
    let middle_bit = 16 - 1 - common_bits;
    let middle: u16 = 1 << middle_bit;

    let mut ports = Vec::new();

    // Wildcard the trailing zeroes to the right of the middle bit of
    // the start. Covers the values immediately following (and incl.)
    // the start. The middle bit is OR'd in to avoid counting zeroes
    // past it.
    let mut bit = (start | middle).trailing_zeros();
    ports.push(masked_port(start, bit));

    // All 0-bits between the trailing zeroes and the middle bit: set
    // the bit and wildcard the lower bits. Covers start → middle not
    // covered above. The current `bit` is 1, so skip it.
    bit += 1;
    while bit < middle_bit {
        if start & (1 << bit) == 0 {
            ports.push(masked_port(start + (1 << bit), bit));
        }
        bit += 1;
    }

    // Wildcard the trailing ones to the right of the middle bit of the
    // end. Covers the values immediately preceding (and incl.) the end.
    bit = (!end | middle).trailing_zeros();
    ports.push(masked_port(end, bit));

    // All 1-bits between the trailing ones and the middle bit: clear the
    // bit and wildcard the lower bits. Covers end → middle not covered
    // above. The current `bit` is 0, so skip it.
    bit += 1;
    while bit < middle_bit {
        if end & (1 << bit) != 0 {
            ports.push(masked_port(end - (1 << bit), bit));
        }
        bit += 1;
    }

    ports
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masked_port_full_wildcard_is_zero_mask() {
        assert_eq!(masked_port(0x1234, 16), MaskedPort { port: 0, mask: 0 });
    }

    #[test]
    fn masked_port_covers_and_matches() {
        // 16-31 prefix.
        let m = MaskedPort {
            port: 0x10,
            mask: 0xfff0,
        };
        assert_eq!(m.covered(), 16);
        assert!(m.matches(16));
        assert!(m.matches(31));
        assert!(!m.matches(15));
        assert!(!m.matches(32));
    }

    #[test]
    fn display_matches_upstream_format() {
        let m = MaskedPort {
            port: 0x10,
            mask: 0xfff0,
        };
        assert_eq!(format!("{m}"), "{port: 0x10, mask: 0xfff0}");
    }
}

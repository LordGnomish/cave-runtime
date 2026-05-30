// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! LogQL `ip()` matcher — ports grafana/loki `pkg/logql/log/ip.go`.
//!
//! Loki's `ip("…")` accepts three pattern forms, both for line filters
//! (`|= ip("…")`) and label filters (`| addr = ip("…")`):
//!   * a single address           — `192.168.4.5`, `::1`
//!   * a CIDR block               — `192.168.0.0/16`, `2001:db8::/32`
//!   * an inclusive range         — `192.168.4.5-192.168.4.20`
//!
//! Matching is family-sensitive: an IPv4 pattern never matches an IPv6
//! address (mirrors Go's `net` semantics, which Loki relies on).

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::IpAddr;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn parse_and_match_single_v4() {
        let p = IpPattern::parse("192.168.4.5").unwrap();
        assert!(p.matches(ip("192.168.4.5")));
        assert!(!p.matches(ip("192.168.4.6")));
    }

    #[test]
    fn parse_and_match_single_v6() {
        let p = IpPattern::parse("::1").unwrap();
        assert!(p.matches(ip("::1")));
        assert!(!p.matches(ip("::2")));
    }

    #[test]
    fn cidr_v4_masks_host_bits() {
        let p = IpPattern::parse("192.168.0.0/16").unwrap();
        assert!(p.matches(ip("192.168.4.5")));
        assert!(p.matches(ip("192.168.255.255")));
        assert!(!p.matches(ip("192.169.0.1")));
    }

    #[test]
    fn cidr_v6_masks_host_bits() {
        let p = IpPattern::parse("2001:db8::/32").unwrap();
        assert!(p.matches(ip("2001:db8::1")));
        assert!(p.matches(ip("2001:db8:ffff::1")));
        assert!(!p.matches(ip("2001:db9::1")));
    }

    #[test]
    fn inclusive_range_v4() {
        let p = IpPattern::parse("192.168.4.5-192.168.4.20").unwrap();
        assert!(p.matches(ip("192.168.4.5")));
        assert!(p.matches(ip("192.168.4.12")));
        assert!(p.matches(ip("192.168.4.20")));
        assert!(!p.matches(ip("192.168.4.4")));
        assert!(!p.matches(ip("192.168.4.21")));
    }

    #[test]
    fn family_mismatch_never_matches() {
        let v4 = IpPattern::parse("192.168.0.0/16").unwrap();
        assert!(!v4.matches(ip("2001:db8::1")));
        let v6 = IpPattern::parse("::1").unwrap();
        assert!(!v6.matches(ip("127.0.0.1")));
    }

    #[test]
    fn rejects_garbage_patterns() {
        assert!(IpPattern::parse("not-an-ip").is_err());
        assert!(IpPattern::parse("192.168.0.0/99").is_err());
        assert!(IpPattern::parse("10.0.0.5-").is_err());
    }
}

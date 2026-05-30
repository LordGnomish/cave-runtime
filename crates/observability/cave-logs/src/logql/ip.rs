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

use std::net::IpAddr;

/// A compiled `ip("…")` pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IpPattern {
    /// A single exact address.
    Single(IpAddr),
    /// A CIDR block: every address sharing `prefix` leading bits with `net`.
    Cidr { net: IpAddr, prefix: u8 },
    /// An inclusive `start-end` range (same family, start ≤ end).
    Range { start: IpAddr, end: IpAddr },
}

/// Maps an address onto a width-tagged 128-bit integer so v4 and v6 never
/// alias: IPv4 keeps its 32-bit value, IPv6 its 128-bit value, and the
/// `is_v6` flag is compared first by every operation here.
fn to_bits(addr: IpAddr) -> (bool, u128) {
    match addr {
        IpAddr::V4(v4) => (false, u32::from(v4) as u128),
        IpAddr::V6(v6) => (true, u128::from(v6)),
    }
}

impl IpPattern {
    /// Parse a Loki `ip()` argument. Returns `Err` for any malformed form.
    pub fn parse(s: &str) -> Result<IpPattern, String> {
        let s = s.trim();
        // Range: `start-end`. A bare `-` (no end) or junk on either side errs.
        if let Some((lhs, rhs)) = s.split_once('-') {
            // Guard against IPv6 (which contains no '-') being misread; both
            // sides must parse as addresses of the same family.
            let start: IpAddr = lhs
                .trim()
                .parse()
                .map_err(|_| format!("invalid range start: {lhs}"))?;
            let end: IpAddr = rhs
                .trim()
                .parse()
                .map_err(|_| format!("invalid range end: {rhs}"))?;
            let (sv6, sbits) = to_bits(start);
            let (ev6, ebits) = to_bits(end);
            if sv6 != ev6 {
                return Err("range endpoints differ in address family".into());
            }
            if sbits > ebits {
                return Err("range start is greater than end".into());
            }
            return Ok(IpPattern::Range { start, end });
        }
        // CIDR: `net/prefix`.
        if let Some((net_str, pfx_str)) = s.split_once('/') {
            let net: IpAddr = net_str
                .trim()
                .parse()
                .map_err(|_| format!("invalid CIDR network: {net_str}"))?;
            let prefix: u8 = pfx_str
                .trim()
                .parse()
                .map_err(|_| format!("invalid CIDR prefix: {pfx_str}"))?;
            let max = if net.is_ipv6() { 128 } else { 32 };
            if prefix > max {
                return Err(format!("CIDR prefix /{prefix} exceeds /{max}"));
            }
            return Ok(IpPattern::Cidr { net, prefix });
        }
        // Single address.
        let addr: IpAddr = s.parse().map_err(|_| format!("invalid address: {s}"))?;
        Ok(IpPattern::Single(addr))
    }

    /// Does `addr` satisfy this pattern? Always `false` across families.
    pub fn matches(&self, addr: IpAddr) -> bool {
        let (av6, abits) = to_bits(addr);
        match self {
            IpPattern::Single(s) => to_bits(*s) == (av6, abits),
            IpPattern::Cidr { net, prefix } => {
                let (nv6, nbits) = to_bits(*net);
                if nv6 != av6 {
                    return false;
                }
                let width = if av6 { 128 } else { 32 };
                if *prefix == 0 {
                    return true;
                }
                // Mask off the host bits and compare the network portions.
                let shift = width - *prefix as u32;
                let mask = if shift >= 128 {
                    0
                } else {
                    u128::MAX << shift
                };
                (nbits & mask) == (abits & mask)
            }
            IpPattern::Range { start, end } => {
                let (sv6, sbits) = to_bits(*start);
                let (_, ebits) = to_bits(*end);
                sv6 == av6 && sbits <= abits && abits <= ebits
            }
        }
    }

    /// Address family this pattern targets (`true` = IPv6).
    fn is_v6(&self) -> bool {
        match self {
            IpPattern::Single(a) => a.is_ipv6(),
            IpPattern::Cidr { net, .. } => net.is_ipv6(),
            IpPattern::Range { start, .. } => start.is_ipv6(),
        }
    }

    /// Scan a raw log line for any embedded address matching this pattern.
    /// The candidate alphabet is family-scoped so a v4 pattern won't snag on
    /// hex-letter runs and a "1.2.3" version string can't parse as an address.
    pub fn line_matches(&self, line: &str) -> bool {
        let v6 = self.is_v6();
        let is_cand = |c: char| {
            if v6 {
                c.is_ascii_hexdigit() || c == ':' || c == '.'
            } else {
                c.is_ascii_digit() || c == '.'
            }
        };
        let mut run = String::new();
        for ch in line.chars() {
            if is_cand(ch) {
                run.push(ch);
            } else if !run.is_empty() {
                if self.run_matches(&run) {
                    return true;
                }
                run.clear();
            }
        }
        !run.is_empty() && self.run_matches(&run)
    }

    /// Try to parse one candidate run as an address and test it, tolerating a
    /// single trailing separator left by adjacent punctuation (`"…4.5."`).
    fn run_matches(&self, run: &str) -> bool {
        if let Ok(addr) = run.parse::<IpAddr>() {
            return self.matches(addr);
        }
        let trimmed = run.trim_end_matches(['.', ':']);
        if trimmed.len() != run.len() {
            if let Ok(addr) = trimmed.parse::<IpAddr>() {
                return self.matches(addr);
            }
        }
        false
    }
}

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

    #[test]
    fn line_match_finds_embedded_v4() {
        let p = IpPattern::parse("192.168.0.0/16").unwrap();
        assert!(p.line_matches(r#"level=info msg="from 192.168.4.5 ok""#));
        assert!(!p.line_matches(r#"level=info msg="from 10.0.0.1 ok""#));
    }

    #[test]
    fn line_match_finds_embedded_v6() {
        let p = IpPattern::parse("2001:db8::/32").unwrap();
        assert!(p.line_matches("client=2001:db8::dead:beef connected"));
        assert!(!p.line_matches("client=fe80::1 connected"));
    }

    #[test]
    fn line_match_range_and_no_ip_line() {
        let p = IpPattern::parse("192.168.4.5-192.168.4.20").unwrap();
        assert!(p.line_matches("a 192.168.4.12 b"));
        assert!(!p.line_matches("a 192.168.4.99 b"));
        // A line with no parseable address never matches.
        assert!(!p.line_matches("no addresses here at all"));
    }

    #[test]
    fn line_match_ignores_version_like_dotted_runs() {
        // "1.2.3" is not a valid IPv4 (3 octets) — must not false-match.
        let p = IpPattern::parse("1.2.0.0/16").unwrap();
        assert!(!p.line_matches("app version 1.2.3 started"));
        assert!(p.line_matches("app talking to 1.2.3.4 now"));
    }
}

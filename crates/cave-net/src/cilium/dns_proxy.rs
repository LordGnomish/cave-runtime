//! DNS proxy — intercepts pod DNS queries for visibility + policy
//! enforcement.
//!
//! Mirrors `pkg/proxy/dns/dnsproxy.go`. When a `toFQDNs` policy is in
//! effect for an endpoint, the dataplane redirects the pod's DNS
//! traffic to the local proxy on a per-endpoint port. The proxy:
//!
//! 1. Parses the question section (qname + qtype).
//! 2. Checks the question against the per-endpoint allow-list.
//! 3. Forwards to the upstream resolver if allowed; drops + returns
//!    REFUSED if denied.
//! 4. Captures the response and pushes the resolved IPs into the
//!    [`super::fqdn::FqdnCache`] so subsequent L4 datapath lookups can
//!    resolve `(pattern, ip) → identity`.
//!
//! Modes (mirror `pkg/option/config.go::DNSProxyMode`):
//!
//! * [`DnsMode::Intercept`] — full proxy (parse + filter + capture).
//! * [`DnsMode::CaptureOnly`] — pass through unmodified but record
//!   the FQDN→IP mapping. Used for visibility-only deployments.

use crate::cilium::types::{Cite, TenantId};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::net::IpAddr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DnsMode {
    Intercept,
    CaptureOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum QType {
    A,
    Aaaa,
    Cname,
    Ns,
    Mx,
    Txt,
    Srv,
    Ptr,
    Other(u16),
}

impl QType {
    /// DNS RR type codes.
    pub fn numeric(self) -> u16 {
        match self {
            QType::A => 1,
            QType::Ns => 2,
            QType::Cname => 5,
            QType::Ptr => 12,
            QType::Mx => 15,
            QType::Txt => 16,
            QType::Aaaa => 28,
            QType::Srv => 33,
            QType::Other(n) => n,
        }
    }
    pub fn from_numeric(n: u16) -> Self {
        match n {
            1 => QType::A,
            2 => QType::Ns,
            5 => QType::Cname,
            12 => QType::Ptr,
            15 => QType::Mx,
            16 => QType::Txt,
            28 => QType::Aaaa,
            33 => QType::Srv,
            other => QType::Other(other),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DnsRcode {
    NoError = 0,
    FormErr = 1,
    ServFail = 2,
    NxDomain = 3,
    NotImp = 4,
    Refused = 5,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DnsQuestion {
    pub qname: String,
    pub qtype: QType,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DnsAnswer {
    pub name: String,
    pub qtype: QType,
    pub ttl_seconds: u32,
    pub data: AnswerData,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AnswerData {
    A(std::net::Ipv4Addr),
    Aaaa(std::net::Ipv6Addr),
    Cname(String),
    Other(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DnsResponse {
    pub rcode: DnsRcode,
    pub answers: Vec<DnsAnswer>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AllowList {
    /// Patterns following `cilium::l7policy::dns_matches` semantics.
    pub patterns: Vec<String>,
}

impl AllowList {
    pub fn allows(&self, name: &str) -> bool {
        self.patterns.iter().any(|p| crate::cilium::l7policy::dns_matches(p, name))
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DnsProxyError {
    #[error("endpoint {0} has no allow-list registered")]
    NoAllowList(u64),
    #[error("malformed query")]
    Malformed,
    #[error("tenant {tenant} cannot mutate DNS proxy owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DnsVerdict {
    Allow,
    Refused,
    PassThrough,
}

#[derive(Debug)]
pub struct DnsProxy {
    pub tenant: TenantId,
    pub mode: DnsMode,
    /// Per-endpoint allow-list.
    allow_lists: HashMap<u64, AllowList>,
    /// Captured (qname, qtype, last_seen_ns).
    captures: BTreeMap<(String, QType), u64>,
    /// Resolved IPs per qname, with TTL anchor.
    resolved: BTreeMap<String, Vec<(IpAddr, u64 /* expiry_ns */)>>,
}

impl DnsProxy {
    pub fn new(tenant: TenantId, mode: DnsMode) -> Self {
        Self {
            tenant, mode,
            allow_lists: HashMap::new(),
            captures: BTreeMap::new(),
            resolved: BTreeMap::new(),
        }
    }

    pub fn set_allow_list(&mut self, endpoint_id: u64, list: AllowList) {
        self.allow_lists.insert(endpoint_id, list);
    }

    pub fn remove_allow_list(&mut self, endpoint_id: u64) -> bool {
        self.allow_lists.remove(&endpoint_id).is_some()
    }

    pub fn allow_list(&self, endpoint_id: u64) -> Option<&AllowList> {
        self.allow_lists.get(&endpoint_id)
    }

    /// Decide what to do with a DNS query from `endpoint_id` for
    /// `question`. Mirrors the proxy decision in
    /// `pkg/proxy/dns/dnsproxy.go::ServeDNS`.
    pub fn on_query(&mut self, endpoint_id: u64, question: &DnsQuestion, now_ns: u64) -> Result<DnsVerdict, DnsProxyError> {
        // Always capture for visibility.
        self.captures.insert((question.qname.clone(), question.qtype), now_ns);
        match self.mode {
            DnsMode::CaptureOnly => Ok(DnsVerdict::PassThrough),
            DnsMode::Intercept => {
                let list = self.allow_lists.get(&endpoint_id)
                    .ok_or(DnsProxyError::NoAllowList(endpoint_id))?;
                if list.allows(&question.qname) {
                    Ok(DnsVerdict::Allow)
                } else {
                    Ok(DnsVerdict::Refused)
                }
            }
        }
    }

    /// Process the upstream response — capture the resolved IPs into
    /// the per-name table, anchored by TTL.
    pub fn on_response(&mut self, question: &DnsQuestion, response: &DnsResponse, now_ns: u64) {
        if !matches!(response.rcode, DnsRcode::NoError) {
            return;
        }
        for ans in &response.answers {
            let ip = match ans.data {
                AnswerData::A(v4) => IpAddr::V4(v4),
                AnswerData::Aaaa(v6) => IpAddr::V6(v6),
                _ => continue,
            };
            let expiry_ns = now_ns + (ans.ttl_seconds as u64) * 1_000_000_000;
            let entries = self.resolved.entry(question.qname.clone()).or_default();
            // De-dup: replace existing entry for the same IP.
            entries.retain(|(existing_ip, _)| *existing_ip != ip);
            entries.push((ip, expiry_ns));
        }
    }

    pub fn lookup_resolved(&self, qname: &str, now_ns: u64) -> Vec<IpAddr> {
        self.resolved.get(qname).map(|v| {
            v.iter().filter(|(_, e)| now_ns < *e).map(|(ip, _)| *ip).collect()
        }).unwrap_or_default()
    }

    pub fn captured_count(&self) -> usize {
        self.captures.len()
    }

    pub fn resolved_name_count(&self) -> usize {
        self.resolved.len()
    }

    /// Reap expired resolved entries. Returns count removed.
    pub fn gc(&mut self, now_ns: u64) -> usize {
        let mut removed = 0;
        let qnames: Vec<String> = self.resolved.keys().cloned().collect();
        for q in qnames {
            if let Some(entries) = self.resolved.get_mut(&q) {
                let before = entries.len();
                entries.retain(|(_, e)| now_ns < *e);
                removed += before - entries.len();
                if entries.is_empty() {
                    self.resolved.remove(&q);
                }
            }
        }
        removed
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/proxy/dns/dnsproxy.go", "DNSProxy");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;
    use std::net::Ipv4Addr;

    fn proxy(tenant: TenantId, mode: DnsMode) -> DnsProxy {
        DnsProxy::new(tenant, mode)
    }

    fn question(qname: &str, qt: QType) -> DnsQuestion {
        DnsQuestion { qname: qname.into(), qtype: qt }
    }

    fn response_a(name: &str, ip: Ipv4Addr, ttl: u32) -> DnsResponse {
        DnsResponse {
            rcode: DnsRcode::NoError,
            answers: vec![DnsAnswer {
                name: name.into(), qtype: QType::A,
                ttl_seconds: ttl, data: AnswerData::A(ip),
            }],
        }
    }

    // ── QType numeric mapping ───────────────────────────────────────────────

    #[test]
    fn qtype_numeric_known_values() {
        let (_c, _t) = cilium_test_ctx!("pkg/proxy/dns/dnsproxy.go", "QType.Numeric", "tenant-dns-qn");
        assert_eq!(QType::A.numeric(), 1);
        assert_eq!(QType::Aaaa.numeric(), 28);
        assert_eq!(QType::Cname.numeric(), 5);
        assert_eq!(QType::Mx.numeric(), 15);
        assert_eq!(QType::Srv.numeric(), 33);
    }

    #[test]
    fn qtype_from_numeric_round_trip_for_known() {
        let (_c, _t) = cilium_test_ctx!("pkg/proxy/dns/dnsproxy.go", "QType.FromNumeric", "tenant-dns-qfn");
        for q in [QType::A, QType::Aaaa, QType::Cname, QType::Mx, QType::Srv, QType::Txt, QType::Ptr, QType::Ns] {
            assert_eq!(QType::from_numeric(q.numeric()), q);
        }
    }

    #[test]
    fn qtype_from_numeric_unknown_falls_to_other() {
        let (_c, _t) = cilium_test_ctx!("pkg/proxy/dns/dnsproxy.go", "QType.Other", "tenant-dns-qother");
        assert_eq!(QType::from_numeric(99), QType::Other(99));
    }

    // ── AllowList ───────────────────────────────────────────────────────────

    #[test]
    fn allow_list_exact_match() {
        let (_c, _t) = cilium_test_ctx!("pkg/proxy/dns/dnsproxy.go", "AllowList.Exact", "tenant-dns-alex");
        let l = AllowList { patterns: vec!["api.example.com".into()] };
        assert!(l.allows("api.example.com"));
        assert!(!l.allows("other.example.com"));
    }

    #[test]
    fn allow_list_wildcard_match() {
        let (_c, _t) = cilium_test_ctx!("pkg/proxy/dns/dnsproxy.go", "AllowList.Wildcard", "tenant-dns-alwc");
        let l = AllowList { patterns: vec!["*.example.com".into()] };
        assert!(l.allows("api.example.com"));
        assert!(l.allows("other.example.com"));
        assert!(!l.allows("example.com"));
    }

    #[test]
    fn allow_list_star_matches_all() {
        let (_c, _t) = cilium_test_ctx!("pkg/proxy/dns/dnsproxy.go", "AllowList.Star", "tenant-dns-alstar");
        let l = AllowList { patterns: vec!["*".into()] };
        assert!(l.allows("anything.example.com"));
    }

    #[test]
    fn allow_list_empty_denies_all() {
        let (_c, _t) = cilium_test_ctx!("pkg/proxy/dns/dnsproxy.go", "AllowList.Empty", "tenant-dns-alempty");
        let l = AllowList { patterns: vec![] };
        assert!(!l.allows("anything"));
    }

    // ── Mode-specific behaviour ─────────────────────────────────────────────

    #[test]
    fn intercept_mode_allow_when_pattern_matches() {
        let (_c, tenant) = cilium_test_ctx!("pkg/proxy/dns/dnsproxy.go", "Intercept.Allow", "tenant-dns-iall");
        let mut p = proxy(tenant, DnsMode::Intercept);
        p.set_allow_list(1, AllowList { patterns: vec!["*.example.com".into()] });
        let v = p.on_query(1, &question("api.example.com", QType::A), 0).unwrap();
        assert_eq!(v, DnsVerdict::Allow);
    }

    #[test]
    fn intercept_mode_refused_when_pattern_does_not_match() {
        let (_c, tenant) = cilium_test_ctx!("pkg/proxy/dns/dnsproxy.go", "Intercept.Refused", "tenant-dns-iref");
        let mut p = proxy(tenant, DnsMode::Intercept);
        p.set_allow_list(1, AllowList { patterns: vec!["*.example.com".into()] });
        let v = p.on_query(1, &question("evil.com", QType::A), 0).unwrap();
        assert_eq!(v, DnsVerdict::Refused);
    }

    #[test]
    fn intercept_mode_no_allow_list_returns_error() {
        let (_c, tenant) = cilium_test_ctx!("pkg/proxy/dns/dnsproxy.go", "Intercept.NoAllowList", "tenant-dns-inal");
        let mut p = proxy(tenant, DnsMode::Intercept);
        let err = p.on_query(99, &question("api.example.com", QType::A), 0).unwrap_err();
        assert_eq!(err, DnsProxyError::NoAllowList(99));
    }

    #[test]
    fn capture_only_mode_pass_through_regardless() {
        let (_c, tenant) = cilium_test_ctx!("pkg/proxy/dns/dnsproxy.go", "CaptureOnly", "tenant-dns-co");
        let mut p = proxy(tenant, DnsMode::CaptureOnly);
        let v = p.on_query(1, &question("evil.com", QType::A), 0).unwrap();
        assert_eq!(v, DnsVerdict::PassThrough);
    }

    #[test]
    fn capture_records_query_regardless_of_verdict() {
        let (_c, tenant) = cilium_test_ctx!("pkg/proxy/dns/dnsproxy.go", "CaptureRecord", "tenant-dns-rec");
        let mut p = proxy(tenant, DnsMode::Intercept);
        p.set_allow_list(1, AllowList { patterns: vec!["*.example.com".into()] });
        let _ = p.on_query(1, &question("evil.com", QType::A), 100);
        let _ = p.on_query(1, &question("api.example.com", QType::A), 200);
        assert_eq!(p.captured_count(), 2);
    }

    // ── Allow-list lifecycle ────────────────────────────────────────────────

    #[test]
    fn allow_list_lookup_returns_set() {
        let (_c, tenant) = cilium_test_ctx!("pkg/proxy/dns/dnsproxy.go", "AllowList.Lookup", "tenant-dns-allk");
        let mut p = proxy(tenant, DnsMode::Intercept);
        let l = AllowList { patterns: vec!["*.example.com".into()] };
        p.set_allow_list(1, l.clone());
        assert_eq!(p.allow_list(1), Some(&l));
    }

    #[test]
    fn allow_list_remove_drops() {
        let (_c, tenant) = cilium_test_ctx!("pkg/proxy/dns/dnsproxy.go", "AllowList.Remove", "tenant-dns-alrm");
        let mut p = proxy(tenant, DnsMode::Intercept);
        p.set_allow_list(1, AllowList { patterns: vec!["*".into()] });
        assert!(p.remove_allow_list(1));
        assert!(p.allow_list(1).is_none());
    }

    #[test]
    fn allow_list_remove_unknown_returns_false() {
        let (_c, tenant) = cilium_test_ctx!("pkg/proxy/dns/dnsproxy.go", "AllowList.Remove.NotFound", "tenant-dns-alrmn");
        let mut p = proxy(tenant, DnsMode::Intercept);
        assert!(!p.remove_allow_list(1));
    }

    // ── Response capture ────────────────────────────────────────────────────

    #[test]
    fn on_response_records_a_record_with_ttl_window() {
        let (_c, tenant) = cilium_test_ctx!("pkg/proxy/dns/dnsproxy.go", "Response.A", "tenant-dns-resa");
        let mut p = proxy(tenant, DnsMode::Intercept);
        p.set_allow_list(1, AllowList { patterns: vec!["*".into()] });
        p.on_response(&question("api.example.com", QType::A),
                      &response_a("api.example.com", Ipv4Addr::new(1, 2, 3, 4), 60),
                      100);
        let r = p.lookup_resolved("api.example.com", 100);
        assert_eq!(r, vec![IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4))]);
    }

    #[test]
    fn on_response_skips_non_noerror_rcode() {
        let (_c, tenant) = cilium_test_ctx!("pkg/proxy/dns/dnsproxy.go", "Response.NXDOMAIN", "tenant-dns-resnx");
        let mut p = proxy(tenant, DnsMode::Intercept);
        let resp = DnsResponse {
            rcode: DnsRcode::NxDomain,
            answers: vec![DnsAnswer {
                name: "api.example.com".into(), qtype: QType::A,
                ttl_seconds: 60, data: AnswerData::A(Ipv4Addr::new(1, 2, 3, 4)),
            }],
        };
        p.on_response(&question("api.example.com", QType::A), &resp, 100);
        assert!(p.lookup_resolved("api.example.com", 100).is_empty());
    }

    #[test]
    fn on_response_dedupes_repeated_ip() {
        let (_c, tenant) = cilium_test_ctx!("pkg/proxy/dns/dnsproxy.go", "Response.Dedup", "tenant-dns-resd");
        let mut p = proxy(tenant, DnsMode::Intercept);
        p.on_response(&question("api.example.com", QType::A),
                      &response_a("api.example.com", Ipv4Addr::new(1, 2, 3, 4), 60), 100);
        p.on_response(&question("api.example.com", QType::A),
                      &response_a("api.example.com", Ipv4Addr::new(1, 2, 3, 4), 120), 200);
        assert_eq!(p.lookup_resolved("api.example.com", 200).len(), 1);
    }

    #[test]
    fn lookup_resolved_filters_expired() {
        let (_c, tenant) = cilium_test_ctx!("pkg/proxy/dns/dnsproxy.go", "LookupResolved.Expiry", "tenant-dns-exp");
        let mut p = proxy(tenant, DnsMode::Intercept);
        p.on_response(&question("api.example.com", QType::A),
                      &response_a("api.example.com", Ipv4Addr::new(1, 2, 3, 4), 60), 0);
        // 60s expiry → at t=70s the entry should be gone from the live view.
        let r = p.lookup_resolved("api.example.com", 70_000_000_000);
        assert!(r.is_empty());
    }

    #[test]
    fn lookup_unknown_qname_returns_empty() {
        let (_c, tenant) = cilium_test_ctx!("pkg/proxy/dns/dnsproxy.go", "LookupResolved.NotFound", "tenant-dns-lkn");
        let p = proxy(tenant, DnsMode::Intercept);
        assert!(p.lookup_resolved("nope.com", 0).is_empty());
    }

    // ── GC ──────────────────────────────────────────────────────────────────

    #[test]
    fn gc_removes_expired_entries() {
        let (_c, tenant) = cilium_test_ctx!("pkg/proxy/dns/dnsproxy.go", "GC.Expired", "tenant-dns-gc");
        let mut p = proxy(tenant, DnsMode::Intercept);
        p.on_response(&question("a.example.com", QType::A),
                      &response_a("a.example.com", Ipv4Addr::new(1, 2, 3, 4), 60), 0);
        p.on_response(&question("b.example.com", QType::A),
                      &response_a("b.example.com", Ipv4Addr::new(1, 2, 3, 5), 600), 0);
        let n = p.gc(70 * 1_000_000_000);
        assert_eq!(n, 1);
        assert_eq!(p.resolved_name_count(), 1);
    }

    #[test]
    fn gc_keeps_fresh_entries() {
        let (_c, tenant) = cilium_test_ctx!("pkg/proxy/dns/dnsproxy.go", "GC.Fresh", "tenant-dns-gcf");
        let mut p = proxy(tenant, DnsMode::Intercept);
        p.on_response(&question("a.example.com", QType::A),
                      &response_a("a.example.com", Ipv4Addr::new(1, 2, 3, 4), 600), 0);
        let n = p.gc(60 * 1_000_000_000);
        assert_eq!(n, 0);
    }

    // ── Multiple endpoints ──────────────────────────────────────────────────

    #[test]
    fn multiple_endpoints_have_independent_allow_lists() {
        let (_c, tenant) = cilium_test_ctx!("pkg/proxy/dns/dnsproxy.go", "AllowList.PerEndpoint", "tenant-dns-multi");
        let mut p = proxy(tenant, DnsMode::Intercept);
        p.set_allow_list(1, AllowList { patterns: vec!["*.example.com".into()] });
        p.set_allow_list(2, AllowList { patterns: vec!["*.other.com".into()] });
        assert_eq!(p.on_query(1, &question("api.example.com", QType::A), 0).unwrap(), DnsVerdict::Allow);
        assert_eq!(p.on_query(2, &question("api.example.com", QType::A), 0).unwrap(), DnsVerdict::Refused);
        assert_eq!(p.on_query(2, &question("api.other.com", QType::A), 0).unwrap(), DnsVerdict::Allow);
    }

    // ── AAAA / CNAME ────────────────────────────────────────────────────────

    #[test]
    fn aaaa_record_captured_as_v6() {
        let (_c, tenant) = cilium_test_ctx!("pkg/proxy/dns/dnsproxy.go", "Response.AAAA", "tenant-dns-aaaa");
        let mut p = proxy(tenant, DnsMode::Intercept);
        let resp = DnsResponse {
            rcode: DnsRcode::NoError,
            answers: vec![DnsAnswer {
                name: "api.example.com".into(), qtype: QType::Aaaa,
                ttl_seconds: 60,
                data: AnswerData::Aaaa("fd00::1".parse().unwrap()),
            }],
        };
        p.on_response(&question("api.example.com", QType::Aaaa), &resp, 0);
        let r = p.lookup_resolved("api.example.com", 0);
        assert!(r.iter().any(|ip| ip.is_ipv6()));
    }

    #[test]
    fn cname_answer_not_captured_as_ip() {
        let (_c, tenant) = cilium_test_ctx!("pkg/proxy/dns/dnsproxy.go", "Response.CNAME", "tenant-dns-cn");
        let mut p = proxy(tenant, DnsMode::Intercept);
        let resp = DnsResponse {
            rcode: DnsRcode::NoError,
            answers: vec![DnsAnswer {
                name: "api.example.com".into(), qtype: QType::Cname,
                ttl_seconds: 60, data: AnswerData::Cname("alias.example.com".into()),
            }],
        };
        p.on_response(&question("api.example.com", QType::Cname), &resp, 0);
        assert!(p.lookup_resolved("api.example.com", 0).is_empty());
    }

    // ── DnsRcode ─────────────────────────────────────────────────────────────

    #[test]
    fn dns_rcode_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/proxy/dns/dnsproxy.go", "Rcode.Serde", "tenant-dns-rcserde");
        for r in [DnsRcode::NoError, DnsRcode::FormErr, DnsRcode::ServFail, DnsRcode::NxDomain, DnsRcode::Refused] {
            let s = serde_json::to_string(&r).unwrap();
            let back: DnsRcode = serde_json::from_str(&s).unwrap();
            assert_eq!(back, r);
        }
    }

    #[test]
    fn dns_question_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/proxy/dns/dnsproxy.go", "Question.Serde", "tenant-dns-qserde");
        let q = question("api.example.com", QType::A);
        let s = serde_json::to_string(&q).unwrap();
        let back: DnsQuestion = serde_json::from_str(&s).unwrap();
        assert_eq!(back, q);
    }

    #[test]
    fn dns_response_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/proxy/dns/dnsproxy.go", "Response.Serde", "tenant-dns-rserde");
        let r = response_a("api.example.com", Ipv4Addr::new(1, 2, 3, 4), 60);
        let s = serde_json::to_string(&r).unwrap();
        let back: DnsResponse = serde_json::from_str(&s).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn dns_mode_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/option/config.go", "DNSProxyMode.Serde", "tenant-dns-mserde");
        for m in [DnsMode::Intercept, DnsMode::CaptureOnly] {
            let s = serde_json::to_string(&m).unwrap();
            let back: DnsMode = serde_json::from_str(&s).unwrap();
            assert_eq!(back, m);
        }
    }

    #[test]
    fn dns_verdict_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/proxy/dns/dnsproxy.go", "Verdict.Serde", "tenant-dns-vserde");
        for v in [DnsVerdict::Allow, DnsVerdict::Refused, DnsVerdict::PassThrough] {
            let s = serde_json::to_string(&v).unwrap();
            let back: DnsVerdict = serde_json::from_str(&s).unwrap();
            assert_eq!(back, v);
        }
    }
}

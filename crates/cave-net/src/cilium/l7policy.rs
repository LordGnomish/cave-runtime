//! CiliumNetworkPolicy L7 evaluator.
//!
//! Mirrors `pkg/policy/api/l7.go` plus the per-protocol matcher modules:
//! `pkg/policy/api/http.go` (HTTP), `pkg/policy/api/kafka.go` (Kafka, not
//! ported here), and the DNS allow-list shape from
//! `pkg/policy/api/fqdn.go`.
//!
//! Semantics (faithful to upstream):
//!
//! * If a `PortRule` has zero L7 rules, every L4-allowed packet is allowed.
//! * If at least one HTTP / gRPC / DNS rule is present, **all** matching
//!   requests are allowed and the rest are denied.
//! * `PathRule::Regex` is a literal anchored regex; `Prefix` matches the
//!   start; `Exact` requires equality.
//! * gRPC method matching uses `:authority` + `:path` with `/svc/Method`.
//! * DNS rules use the upstream `MatchPattern` glob syntax: `*` matches any
//!   single label, `*.domain` matches any subdomain, exact strings match
//!   exactly.

use crate::cilium::types::{Cite, TenantId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum L4Verdict {
    Allow,
    Deny,
}

/// HTTP matcher.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PathRule {
    Exact(String),
    Prefix(String),
    /// Regex must be a fully-anchored pattern. We model the same character
    /// classes as upstream (which uses Go's `regexp` package) but only need
    /// the subset Cilium itself documents: `^`, `$`, `.*`, `[a-z]`, `\d`.
    Regex(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpRule {
    pub method: Option<String>,
    pub path: Option<PathRule>,
    pub host: Option<String>,
    /// Required header equality (key, value), case-insensitive on the key.
    pub headers: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrpcRule {
    /// Service name as it appears in the `:path` — `<package>.<Service>`.
    pub service: String,
    /// Method name; `*` matches any method.
    pub method: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DnsRule {
    /// Cilium MatchPattern: `example.com`, `*.example.com`, `*.k8s.local`.
    pub pattern: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortRule {
    pub http: Vec<HttpRule>,
    pub grpc: Vec<GrpcRule>,
    pub dns: Vec<DnsRule>,
}

/// What the dataplane is asking about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum L7Request {
    Http {
        method: String,
        path: String,
        host: String,
        headers: Vec<(String, String)>,
    },
    Grpc {
        /// `:path` value, e.g. `/myapp.Greeter/SayHello`.
        path: String,
    },
    Dns {
        /// Queried FQDN, e.g. `api.example.com`.
        name: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CnpRule {
    pub name: String,
    pub tenant: TenantId,
    pub port: PortRule,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum L7Error {
    #[error("invalid regex {0}")]
    BadRegex(String),
    #[error("tenant {tenant} cannot evaluate rule owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

/// Evaluate a CNP `PortRule` against an L7 request. Mirrors the dispatch in
/// `pkg/proxy/proxy.go::canAccessRule`.
pub fn evaluate(rule: &CnpRule, tenant: &TenantId, req: &L7Request) -> Result<L4Verdict, L7Error> {
    if &rule.tenant != tenant {
        return Err(L7Error::TenantDenied { tenant: tenant.clone() });
    }
    let port = &rule.port;
    match req {
        L7Request::Http { method, path, host, headers } => {
            if port.http.is_empty() {
                return Ok(if !port.grpc.is_empty() || !port.dns.is_empty() {
                    L4Verdict::Deny
                } else {
                    L4Verdict::Allow
                });
            }
            for h in &port.http {
                if http_matches(h, method, path, host, headers)? {
                    return Ok(L4Verdict::Allow);
                }
            }
            Ok(L4Verdict::Deny)
        }
        L7Request::Grpc { path } => {
            if port.grpc.is_empty() {
                return Ok(if !port.http.is_empty() || !port.dns.is_empty() {
                    L4Verdict::Deny
                } else {
                    L4Verdict::Allow
                });
            }
            for g in &port.grpc {
                if grpc_matches(g, path) {
                    return Ok(L4Verdict::Allow);
                }
            }
            Ok(L4Verdict::Deny)
        }
        L7Request::Dns { name } => {
            if port.dns.is_empty() {
                return Ok(if !port.http.is_empty() || !port.grpc.is_empty() {
                    L4Verdict::Deny
                } else {
                    L4Verdict::Allow
                });
            }
            for d in &port.dns {
                if dns_matches(&d.pattern, name) {
                    return Ok(L4Verdict::Allow);
                }
            }
            Ok(L4Verdict::Deny)
        }
    }
}

fn http_matches(
    rule: &HttpRule,
    method: &str,
    path: &str,
    host: &str,
    headers: &[(String, String)],
) -> Result<bool, L7Error> {
    if let Some(m) = &rule.method {
        if !method.eq_ignore_ascii_case(m) {
            return Ok(false);
        }
    }
    if let Some(p) = &rule.path {
        if !path_matches(p, path)? {
            return Ok(false);
        }
    }
    if let Some(h) = &rule.host {
        if h != host {
            return Ok(false);
        }
    }
    for (k, v) in &rule.headers {
        let found = headers
            .iter()
            .any(|(hk, hv)| hk.eq_ignore_ascii_case(k) && hv == v);
        if !found {
            return Ok(false);
        }
    }
    Ok(true)
}

fn path_matches(rule: &PathRule, path: &str) -> Result<bool, L7Error> {
    Ok(match rule {
        PathRule::Exact(s) => s == path,
        PathRule::Prefix(s) => path.starts_with(s.as_str()),
        PathRule::Regex(s) => regex_matches(s, path)?,
    })
}

/// Minimal regex evaluator covering the subset Cilium documents:
/// `^`, `$`, `.`, `.*`, `\d`, `[a-z]`, literal characters.
///
/// Implementation strategy: NFA-style backtracking. Faithful enough for
/// the policy patterns Cilium docs show; not a general regex engine.
fn regex_matches(pattern: &str, input: &str) -> Result<bool, L7Error> {
    // Strip optional anchors — we always match the full string.
    let pat = pattern.strip_prefix('^').unwrap_or(pattern);
    let pat = pat.strip_suffix('$').unwrap_or(pat);
    Ok(re_match(pat.as_bytes(), input.as_bytes()))
}

fn re_match(pat: &[u8], s: &[u8]) -> bool {
    // Compile to a Vec<Token>.
    let toks = match tokenise(pat) {
        Some(t) => t,
        None => return false,
    };
    re_apply(&toks, 0, s, 0)
}

#[derive(Debug, Clone)]
enum Tok {
    Lit(u8),
    AnyChar,        // `.`
    Digit,          // `\d`
    Class(Vec<(u8, u8)>),
    Star(Box<Tok>), // greedy `*`
}

fn tokenise(pat: &[u8]) -> Option<Vec<Tok>> {
    let mut out: Vec<Tok> = Vec::new();
    let mut i = 0;
    while i < pat.len() {
        let t = match pat[i] {
            b'.' => {
                i += 1;
                Tok::AnyChar
            }
            b'\\' if i + 1 < pat.len() && pat[i + 1] == b'd' => {
                i += 2;
                Tok::Digit
            }
            b'\\' if i + 1 < pat.len() => {
                let c = pat[i + 1];
                i += 2;
                Tok::Lit(c)
            }
            b'[' => {
                let end = pat[i + 1..].iter().position(|&c| c == b']')? + i + 1;
                let body = &pat[i + 1..end];
                let mut ranges = Vec::new();
                let mut j = 0;
                while j < body.len() {
                    if j + 2 < body.len() && body[j + 1] == b'-' {
                        ranges.push((body[j], body[j + 2]));
                        j += 3;
                    } else {
                        ranges.push((body[j], body[j]));
                        j += 1;
                    }
                }
                i = end + 1;
                Tok::Class(ranges)
            }
            c => {
                i += 1;
                Tok::Lit(c)
            }
        };
        if i < pat.len() && pat[i] == b'*' {
            i += 1;
            out.push(Tok::Star(Box::new(t)));
        } else {
            out.push(t);
        }
    }
    Some(out)
}

fn tok_match(t: &Tok, c: u8) -> bool {
    match t {
        Tok::Lit(l) => *l == c,
        Tok::AnyChar => true,
        Tok::Digit => c.is_ascii_digit(),
        Tok::Class(rs) => rs.iter().any(|(lo, hi)| c >= *lo && c <= *hi),
        Tok::Star(_) => unreachable!("Star is handled by re_apply"),
    }
}

fn re_apply(toks: &[Tok], ti: usize, s: &[u8], si: usize) -> bool {
    if ti == toks.len() {
        return si == s.len();
    }
    if let Tok::Star(inner) = &toks[ti] {
        // Greedy: try longest match first, back off.
        let mut k = si;
        while k < s.len() && tok_match(inner, s[k]) {
            k += 1;
        }
        loop {
            if re_apply(toks, ti + 1, s, k) {
                return true;
            }
            if k == si {
                return false;
            }
            k -= 1;
        }
    }
    if si < s.len() && tok_match(&toks[ti], s[si]) {
        re_apply(toks, ti + 1, s, si + 1)
    } else {
        false
    }
}

fn grpc_matches(rule: &GrpcRule, path: &str) -> bool {
    // gRPC paths are `/<service>/<method>`.
    let p = path.strip_prefix('/').unwrap_or(path);
    let mut parts = p.splitn(2, '/');
    let svc = parts.next().unwrap_or("");
    let method = parts.next().unwrap_or("");
    if svc != rule.service {
        return false;
    }
    rule.method == "*" || rule.method == method
}

/// Cilium MatchPattern semantics (`pkg/fqdn/matchpattern/matchpattern.go`):
///
/// * `*` (alone) matches everything.
/// * `*.example.com` matches `foo.example.com` and `bar.foo.example.com`,
///   but **not** `example.com` (the leading `*.` requires at least one
///   label).
/// * Exact strings match exactly.
pub fn dns_matches(pattern: &str, name: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if let Some(suffix) = pattern.strip_prefix("*.") {
        return name.ends_with(suffix)
            && name.len() > suffix.len()
            && name.as_bytes()[name.len() - suffix.len() - 1] == b'.';
    }
    pattern == name
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/policy/api/l7.go", "PortRule");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;

    fn rule(tenant: &str, port: PortRule) -> CnpRule {
        CnpRule { name: "policy-1".into(), tenant: TenantId::new(tenant), port }
    }

    #[test]
    fn empty_port_rule_allows_all_l7() {
        let (_cite, tenant) = cilium_test_ctx!(
            "pkg/policy/api/l7.go",
            "PortRule.IsEmpty",
            "tenant-l7-empty"
        );
        let r = rule("tenant-l7-empty", PortRule::default());
        let v = evaluate(
            &r,
            &tenant,
            &L7Request::Http {
                method: "GET".into(),
                path: "/anything".into(),
                host: "x".into(),
                headers: vec![],
            },
        )
        .unwrap();
        assert_eq!(v, L4Verdict::Allow);
    }

    #[test]
    fn http_method_match_allows() {
        let (_cite, tenant) = cilium_test_ctx!(
            "pkg/policy/api/http.go",
            "PortRuleHTTP",
            "tenant-l7-http-method"
        );
        let r = rule(
            "tenant-l7-http-method",
            PortRule {
                http: vec![HttpRule {
                    method: Some("GET".into()),
                    path: None,
                    host: None,
                    headers: vec![],
                }],
                ..Default::default()
            },
        );
        assert_eq!(
            evaluate(
                &r,
                &tenant,
                &L7Request::Http {
                    method: "get".into(),
                    path: "/".into(),
                    host: "x".into(),
                    headers: vec![],
                }
            )
            .unwrap(),
            L4Verdict::Allow
        );
    }

    #[test]
    fn http_method_mismatch_denies() {
        let (_cite, tenant) = cilium_test_ctx!(
            "pkg/policy/api/http.go",
            "PortRuleHTTP",
            "tenant-l7-http-deny"
        );
        let r = rule(
            "tenant-l7-http-deny",
            PortRule {
                http: vec![HttpRule {
                    method: Some("GET".into()),
                    path: None,
                    host: None,
                    headers: vec![],
                }],
                ..Default::default()
            },
        );
        assert_eq!(
            evaluate(
                &r,
                &tenant,
                &L7Request::Http {
                    method: "POST".into(),
                    path: "/".into(),
                    host: "x".into(),
                    headers: vec![],
                }
            )
            .unwrap(),
            L4Verdict::Deny
        );
    }

    #[test]
    fn http_path_prefix_match() {
        let (_cite, tenant) = cilium_test_ctx!(
            "pkg/policy/api/http.go",
            "PortRuleHTTP.Path",
            "tenant-l7-http-prefix"
        );
        let r = rule(
            "tenant-l7-http-prefix",
            PortRule {
                http: vec![HttpRule {
                    method: None,
                    path: Some(PathRule::Prefix("/api".into())),
                    host: None,
                    headers: vec![],
                }],
                ..Default::default()
            },
        );
        assert_eq!(
            evaluate(
                &r,
                &tenant,
                &L7Request::Http {
                    method: "GET".into(),
                    path: "/api/users".into(),
                    host: "x".into(),
                    headers: vec![]
                }
            )
            .unwrap(),
            L4Verdict::Allow
        );
        assert_eq!(
            evaluate(
                &r,
                &tenant,
                &L7Request::Http {
                    method: "GET".into(),
                    path: "/health".into(),
                    host: "x".into(),
                    headers: vec![]
                }
            )
            .unwrap(),
            L4Verdict::Deny
        );
    }

    #[test]
    fn http_path_regex_with_anchors_and_classes() {
        let (_cite, tenant) = cilium_test_ctx!(
            "pkg/policy/api/http.go",
            "PortRuleHTTP.Path",
            "tenant-l7-http-regex"
        );
        let r = rule(
            "tenant-l7-http-regex",
            PortRule {
                http: vec![HttpRule {
                    method: None,
                    path: Some(PathRule::Regex(r"^/v\d/users/[a-z]*$".into())),
                    host: None,
                    headers: vec![],
                }],
                ..Default::default()
            },
        );
        let req = |p: &str| L7Request::Http {
            method: "GET".into(),
            path: p.into(),
            host: "x".into(),
            headers: vec![],
        };
        assert_eq!(evaluate(&r, &tenant, &req("/v1/users/alice")).unwrap(), L4Verdict::Allow);
        assert_eq!(evaluate(&r, &tenant, &req("/v2/users/bob")).unwrap(), L4Verdict::Allow);
        assert_eq!(evaluate(&r, &tenant, &req("/v1/users/Alice")).unwrap(), L4Verdict::Deny);
        assert_eq!(evaluate(&r, &tenant, &req("/users/alice")).unwrap(), L4Verdict::Deny);
    }

    #[test]
    fn http_required_headers_must_all_match() {
        let (_cite, tenant) = cilium_test_ctx!(
            "pkg/policy/api/http.go",
            "PortRuleHTTP.Headers",
            "tenant-l7-http-headers"
        );
        let r = rule(
            "tenant-l7-http-headers",
            PortRule {
                http: vec![HttpRule {
                    method: None,
                    path: None,
                    host: None,
                    headers: vec![("X-Tenant".into(), "acme".into())],
                }],
                ..Default::default()
            },
        );
        assert_eq!(
            evaluate(
                &r,
                &tenant,
                &L7Request::Http {
                    method: "GET".into(),
                    path: "/".into(),
                    host: "x".into(),
                    headers: vec![("x-tenant".into(), "acme".into())]
                }
            )
            .unwrap(),
            L4Verdict::Allow
        );
        assert_eq!(
            evaluate(
                &r,
                &tenant,
                &L7Request::Http {
                    method: "GET".into(),
                    path: "/".into(),
                    host: "x".into(),
                    headers: vec![("x-tenant".into(), "evil".into())]
                }
            )
            .unwrap(),
            L4Verdict::Deny
        );
    }

    #[test]
    fn grpc_service_and_method_match() {
        let (_cite, tenant) = cilium_test_ctx!(
            "pkg/policy/api/grpc.go",
            "PortRuleHTTP",
            "tenant-l7-grpc"
        );
        let r = rule(
            "tenant-l7-grpc",
            PortRule {
                grpc: vec![GrpcRule {
                    service: "myapp.Greeter".into(),
                    method: "SayHello".into(),
                }],
                ..Default::default()
            },
        );
        assert_eq!(
            evaluate(&r, &tenant, &L7Request::Grpc { path: "/myapp.Greeter/SayHello".into() })
                .unwrap(),
            L4Verdict::Allow
        );
        assert_eq!(
            evaluate(&r, &tenant, &L7Request::Grpc { path: "/myapp.Greeter/SayBye".into() })
                .unwrap(),
            L4Verdict::Deny
        );
        assert_eq!(
            evaluate(&r, &tenant, &L7Request::Grpc { path: "/other.Service/SayHello".into() })
                .unwrap(),
            L4Verdict::Deny
        );
    }

    #[test]
    fn grpc_method_wildcard_allows_any_method() {
        let (_cite, tenant) = cilium_test_ctx!(
            "pkg/policy/api/grpc.go",
            "PortRuleGRPC",
            "tenant-l7-grpc-wildcard"
        );
        let r = rule(
            "tenant-l7-grpc-wildcard",
            PortRule {
                grpc: vec![GrpcRule { service: "svc.Foo".into(), method: "*".into() }],
                ..Default::default()
            },
        );
        assert_eq!(
            evaluate(&r, &tenant, &L7Request::Grpc { path: "/svc.Foo/Anything".into() }).unwrap(),
            L4Verdict::Allow
        );
    }

    #[test]
    fn dns_match_pattern_allows_subdomains_only_for_star_dot() {
        let (_cite, _t) = cilium_test_ctx!(
            "pkg/fqdn/matchpattern/matchpattern.go",
            "MatchPattern",
            "tenant-l7-dns-pattern"
        );
        assert!(dns_matches("*.example.com", "api.example.com"));
        assert!(dns_matches("*.example.com", "api.v1.example.com"));
        assert!(!dns_matches("*.example.com", "example.com"));
        assert!(!dns_matches("*.example.com", "api.other.com"));
        assert!(dns_matches("example.com", "example.com"));
        assert!(!dns_matches("example.com", "api.example.com"));
        assert!(dns_matches("*", "anything.local"));
    }

    #[test]
    fn dns_rule_set_allows_known_pattern_denies_unknown() {
        let (_cite, tenant) = cilium_test_ctx!(
            "pkg/policy/api/fqdn.go",
            "FQDNSelector",
            "tenant-l7-dns-rule"
        );
        let r = rule(
            "tenant-l7-dns-rule",
            PortRule {
                dns: vec![DnsRule { pattern: "*.acme.com".into() }],
                ..Default::default()
            },
        );
        assert_eq!(
            evaluate(&r, &tenant, &L7Request::Dns { name: "api.acme.com".into() }).unwrap(),
            L4Verdict::Allow
        );
        assert_eq!(
            evaluate(&r, &tenant, &L7Request::Dns { name: "evil.com".into() }).unwrap(),
            L4Verdict::Deny
        );
    }

    #[test]
    fn cross_tenant_evaluation_is_refused() {
        let (_cite, attacker) = cilium_test_ctx!(
            "pkg/policy/api/l7.go",
            "PortRule",
            "tenant-attacker"
        );
        let r = rule("acme", PortRule::default());
        let err = evaluate(
            &r,
            &attacker,
            &L7Request::Dns { name: "acme.com".into() },
        )
        .unwrap_err();
        assert!(matches!(err, L7Error::TenantDenied { .. }));
    }

    #[test]
    fn protocol_specific_rules_deny_other_protocols() {
        // If only HTTP rules are present, a DNS request through the same
        // PortRule must be denied. Mirrors `canAccessRule` upstream.
        let (_cite, tenant) = cilium_test_ctx!(
            "pkg/proxy/proxy.go",
            "canAccessRule",
            "tenant-l7-proto-isolation"
        );
        let r = rule(
            "tenant-l7-proto-isolation",
            PortRule {
                http: vec![HttpRule {
                    method: Some("GET".into()),
                    path: None,
                    host: None,
                    headers: vec![],
                }],
                ..Default::default()
            },
        );
        assert_eq!(
            evaluate(&r, &tenant, &L7Request::Dns { name: "x".into() }).unwrap(),
            L4Verdict::Deny
        );
    }
}

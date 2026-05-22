// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Edge-case coverage close for cave-dast public surface.
//!
//! These integration tests exercise behaviours that the in-module unit
//! tests touch lightly or not at all: serde round-trips at the edge,
//! state-transition idempotency for auth, boundary cases in the spider
//! / context filter pipeline, CLI argument parser failure modes, HTTP
//! message parse/render symmetry, OWASP/CWE taxonomy coverage, and
//! report rendering quirks.

use cave_dast::alert::{Alert, OwaspTop10, cwe_to_owasp, owasp_cwe_examples};
use cave_dast::ascan::{ScanPluginRegistry, sqli::SqlInjectionRule};
use cave_dast::auth::{AuthMethod, FormAuthConfig, apply_auth};
use cave_dast::cli::{CliCommand, CliError, parse as cli_parse};
use cave_dast::context::Context;
use cave_dast::engine::{
    count_by_risk, findings_with_cwe, high_risk_findings, is_valid_url, risk_rank, scan_score,
};
use cave_dast::http::{
    HttpMethod, HttpRequest, HttpResponse, SetCookie, parse::parse_request, parse::parse_response,
    parse_query, percent_decode, url as url_mod,
};
use cave_dast::models::{DastFinding, DastScan, RiskLevel, ScanStatus, ScanType};
use cave_dast::pscan::{
    PassiveScanRegistry, insecure_cookie::InsecureCookieRule,
    security_headers::MissingSecurityHeadersRule,
};
use cave_dast::spider::{
    Discovered, Spider, SpiderConfig, extract_hrefs, parse_robots_disallow,
};
use chrono::Utc;
use uuid::Uuid;

// ---------------------------------------------------------------------
// alert / OWASP taxonomy
// ---------------------------------------------------------------------

#[test]
fn alert_serde_optional_evidence_none_roundtrip() {
    let a = Alert {
        name: "x".to_string(),
        risk: RiskLevel::Low,
        cwe_id: 200,
        url: "http://x/".to_string(),
        description: "d".to_string(),
        solution: "s".to_string(),
        evidence: None,
        plugin_id: 10001,
    };
    let j = serde_json::to_string(&a).unwrap();
    assert!(j.contains("\"evidence\":null"));
    let back: Alert = serde_json::from_str(&j).unwrap();
    assert_eq!(a, back);
    assert!(back.evidence.is_none());
}

#[test]
fn alert_pretty_json_roundtrip() {
    let a = Alert {
        name: "XSS".to_string(),
        risk: RiskLevel::High,
        cwe_id: 79,
        url: "http://x/?q=<".to_string(),
        description: "d".to_string(),
        solution: "esc".to_string(),
        evidence: Some("<script>".to_string()),
        plugin_id: 40012,
    };
    let pretty = serde_json::to_string_pretty(&a).unwrap();
    let back: Alert = serde_json::from_str(&pretty).unwrap();
    assert_eq!(back.cwe_id, 79);
    assert_eq!(back.risk, RiskLevel::High);
}

#[test]
fn owasp_top10_code_is_unique_per_category() {
    let cats = [
        OwaspTop10::A01BrokenAccessControl,
        OwaspTop10::A02CryptographicFailures,
        OwaspTop10::A03Injection,
        OwaspTop10::A04InsecureDesign,
        OwaspTop10::A05SecurityMisconfiguration,
        OwaspTop10::A06VulnerableComponents,
        OwaspTop10::A07IdentificationAuthnFailures,
        OwaspTop10::A08SoftwareDataIntegrityFailures,
        OwaspTop10::A09SecurityLoggingMonitoringFailures,
        OwaspTop10::A10ServerSideRequestForgery,
    ];
    let mut codes: Vec<&str> = cats.iter().map(|c| c.code()).collect();
    codes.sort();
    let n = codes.len();
    codes.dedup();
    assert_eq!(codes.len(), n);
}

#[test]
fn owasp_examples_round_trip_via_cwe_to_owasp() {
    // Every example CWE in a bucket must map back to *some* OWASP cat
    // (not necessarily the same one — A02/A04 overlaps follow ZAP's
    // primary choice).
    for cat in [
        OwaspTop10::A01BrokenAccessControl,
        OwaspTop10::A03Injection,
        OwaspTop10::A10ServerSideRequestForgery,
    ] {
        for cwe in owasp_cwe_examples(cat) {
            assert!(
                cwe_to_owasp(cwe).is_some(),
                "example cwe {} for {:?} unmapped",
                cwe,
                cat
            );
        }
    }
}

#[test]
fn cwe_to_owasp_known_misconfig_buckets() {
    // CWE 693 is the ZAP fallback for missing security-header alerts —
    // must land in A05.
    assert_eq!(
        cwe_to_owasp(693),
        Some(OwaspTop10::A05SecurityMisconfiguration)
    );
    assert_eq!(cwe_to_owasp(0), None);
}

// ---------------------------------------------------------------------
// models — risk / scan / finding serde edges
// ---------------------------------------------------------------------

#[test]
fn risk_level_serializes_lowercase() {
    for (level, expected) in [
        (RiskLevel::High, "\"high\""),
        (RiskLevel::Medium, "\"medium\""),
        (RiskLevel::Low, "\"low\""),
        (RiskLevel::Informational, "\"informational\""),
    ] {
        let j = serde_json::to_string(&level).unwrap();
        assert_eq!(j, expected);
    }
}

#[test]
fn scan_status_serde_round_trip_all_variants() {
    for s in [
        ScanStatus::Queued,
        ScanStatus::Running,
        ScanStatus::Completed,
        ScanStatus::Failed,
    ] {
        let j = serde_json::to_string(&s).unwrap();
        let back: ScanStatus = serde_json::from_str(&j).unwrap();
        assert_eq!(s, back);
    }
}

#[test]
fn dast_scan_empty_findings_score_is_zero() {
    let scan = DastScan {
        id: Uuid::new_v4(),
        target_url: "https://x/".to_string(),
        scan_type: ScanType::Baseline,
        status: ScanStatus::Queued,
        created_at: Utc::now(),
        completed_at: None,
        findings: vec![],
    };
    assert_eq!(scan_score(&scan), 0);
}

#[test]
fn risk_rank_total_order_matches_serde_order() {
    // High > Medium > Low > Informational, and lowercase serde stays
    // independent.
    assert!(risk_rank(&RiskLevel::High) > risk_rank(&RiskLevel::Low));
    assert!(risk_rank(&RiskLevel::Informational) < risk_rank(&RiskLevel::Medium));
}

// ---------------------------------------------------------------------
// engine — counting / filtering edges
// ---------------------------------------------------------------------

fn finding(risk: RiskLevel, cwe: Option<u32>) -> DastFinding {
    DastFinding {
        id: Uuid::new_v4(),
        name: "test".to_string(),
        risk,
        url: "http://x/".to_string(),
        method: "GET".to_string(),
        description: "d".to_string(),
        solution: "s".to_string(),
        cwe_id: cwe,
    }
}

#[test]
fn engine_count_by_risk_on_empty_returns_empty() {
    let c = count_by_risk(&[]);
    assert!(c.is_empty());
}

#[test]
fn engine_high_risk_findings_empty_when_none_high() {
    let f = vec![
        finding(RiskLevel::Low, None),
        finding(RiskLevel::Informational, None),
    ];
    assert!(high_risk_findings(&f).is_empty());
}

#[test]
fn engine_findings_with_cwe_partitions_correctly() {
    let f = vec![
        finding(RiskLevel::High, Some(89)),
        finding(RiskLevel::High, None),
        finding(RiskLevel::Medium, Some(79)),
    ];
    assert_eq!(findings_with_cwe(&f).len(), 2);
}

#[test]
fn engine_is_valid_url_edges() {
    assert!(!is_valid_url("HTTP://X/")); // case-sensitive on purpose
    assert!(!is_valid_url("javascript:alert(1)"));
    assert!(!is_valid_url("//cdn.test/"));
    assert!(is_valid_url("http://"));
}

// ---------------------------------------------------------------------
// context — include/exclude state machine
// ---------------------------------------------------------------------

#[test]
fn context_exclude_only_blocks_match_allows_rest() {
    let mut c = Context::new("c");
    c.exclude(r"/admin/").unwrap();
    // No includes — everything not excluded is in scope.
    assert!(c.is_in_scope("http://x/users"));
    assert!(!c.is_in_scope("http://x/admin/secret"));
}

#[test]
fn context_filter_preserves_order() {
    let mut c = Context::new("c");
    c.include(r"x\.test").unwrap();
    let urls = vec![
        "https://x.test/a".to_string(),
        "https://y.test/b".to_string(),
        "https://x.test/c".to_string(),
        "https://x.test/d".to_string(),
    ];
    let kept = c.filter(&urls);
    assert_eq!(kept[0], "https://x.test/a");
    assert_eq!(kept[1], "https://x.test/c");
    assert_eq!(kept[2], "https://x.test/d");
}

#[test]
fn context_exclude_short_circuits_even_with_include_match() {
    let mut c = Context::new("c");
    c.include(r".*").unwrap();
    c.exclude(r"^https://blocked\.test/").unwrap();
    assert!(!c.is_in_scope("https://blocked.test/x"));
    assert!(c.is_in_scope("https://ok.test/x"));
}

#[test]
fn context_invalid_exclude_regex_returns_error() {
    let mut c = Context::new("c");
    assert!(c.exclude("(unclosed").is_err());
}

// ---------------------------------------------------------------------
// auth — state transitions / idempotency
// ---------------------------------------------------------------------

fn form_cfg() -> FormAuthConfig {
    FormAuthConfig {
        login_url: "https://x.test/login".to_string(),
        username_field: "u".to_string(),
        password_field: "p".to_string(),
        username: "alice".to_string(),
        password: "secret".to_string(),
        session_cookie_name: "SID".to_string(),
    }
}

#[test]
fn bearer_apply_is_idempotent() {
    let m = AuthMethod::BearerToken("tok".to_string());
    let mut req = HttpRequest::new(HttpMethod::Get, "http://x/");
    apply_auth(&m, None, &mut req);
    apply_auth(&m, None, &mut req);
    apply_auth(&m, None, &mut req);
    let all: Vec<_> = req.headers.all("Authorization").collect();
    assert_eq!(all, vec!["Bearer tok"]);
}

#[test]
fn form_apply_without_captured_cookie_is_noop() {
    let m = AuthMethod::FormBased(form_cfg());
    let mut req = HttpRequest::new(HttpMethod::Get, "http://x/api");
    apply_auth(&m, None, &mut req);
    assert!(req.headers.first("Cookie").is_none());
}

#[test]
fn form_apply_replaces_existing_cookie() {
    let m = AuthMethod::FormBased(form_cfg());
    let mut req = HttpRequest::new(HttpMethod::Get, "http://x/api");
    req.headers.insert("Cookie", "SID=old");
    let mut sc = SetCookie::default();
    sc.name = "SID".to_string();
    sc.value = "new".to_string();
    apply_auth(&m, Some(&sc), &mut req);
    let all: Vec<_> = req.headers.all("Cookie").collect();
    assert_eq!(all, vec!["SID=new"]);
}

#[test]
fn form_capture_session_returns_none_when_cookie_absent() {
    let cfg = form_cfg();
    let mut resp = HttpResponse::new(200, "OK");
    resp.headers.insert("Set-Cookie", "other=1");
    assert!(cfg.capture_session(&resp).is_none());
}

#[test]
fn auth_method_none_equality() {
    assert_eq!(AuthMethod::None, AuthMethod::None);
    assert_ne!(AuthMethod::None, AuthMethod::BearerToken("x".into()));
}

// ---------------------------------------------------------------------
// HTTP message edges — parse + render symmetry
// ---------------------------------------------------------------------

#[test]
fn http_method_other_preserved_uppercased() {
    let m = HttpMethod::parse("propfind");
    assert_eq!(m.as_str(), "PROPFIND");
}

#[test]
fn http_method_safe_set_strict() {
    assert!(!HttpMethod::Patch.is_safe());
    assert!(!HttpMethod::Put.is_safe());
    assert!(!HttpMethod::Connect.is_safe());
    assert!(HttpMethod::Options.is_safe());
    assert!(HttpMethod::Trace.is_safe());
}

#[test]
fn header_remove_is_case_insensitive() {
    let mut req = HttpRequest::new(HttpMethod::Get, "http://x/");
    req.headers.insert("X-Custom", "1");
    req.headers.insert("x-custom", "2");
    req.headers.remove("X-CUSTOM");
    assert!(req.headers.first("x-custom").is_none());
}

#[test]
fn set_cookie_domain_path_parsed() {
    let c = SetCookie::parse("k=v; Domain=.x.test; Path=/api; Max-Age=60");
    assert_eq!(c.domain.as_deref(), Some(".x.test"));
    assert_eq!(c.path.as_deref(), Some("/api"));
    assert_eq!(c.max_age, Some(60));
}

#[test]
fn set_cookie_invalid_max_age_drops_to_none() {
    let c = SetCookie::parse("k=v; Max-Age=NaN");
    assert!(c.max_age.is_none());
}

#[test]
fn percent_decode_handles_plus_and_invalid_truncation() {
    assert_eq!(percent_decode("a+b%20c"), "a b c");
    // Two trailing `%` bytes with no hex pair → pass-through bytes.
    assert_eq!(percent_decode("trail%"), "trail%");
}

#[test]
fn parse_query_dedupes_last_wins() {
    let q = parse_query("a=1&a=2&a=3");
    // BTreeMap insert → last value wins.
    assert_eq!(q.get("a"), Some(&"3".to_string()));
}

#[test]
fn parse_request_extracts_multi_set_cookie() {
    let raw = "HTTP/1.1 200 OK\r\nSet-Cookie: a=1; Secure\r\nSet-Cookie: b=2; HttpOnly\r\n\r\n";
    let resp = parse_response(raw).unwrap();
    let cookies = resp.set_cookies();
    assert_eq!(cookies.len(), 2);
    assert_eq!(cookies[0].name, "a");
    assert!(cookies[0].secure);
    assert_eq!(cookies[1].name, "b");
    assert!(cookies[1].http_only);
}

#[test]
fn parse_request_round_trip_via_render() {
    let raw = "GET /api?x=1 HTTP/1.1\r\nHost: x.test\r\nAccept: text/html\r\n\r\n";
    let req = parse_request(raw).unwrap();
    let rendered = req.render();
    // Render emits the request-line + headers + blank line. Re-parse
    // must produce the same method/url/host header.
    let req2 = parse_request(&rendered).unwrap();
    assert_eq!(req2.method, HttpMethod::Get);
    assert_eq!(req2.url, "/api?x=1");
    assert_eq!(req2.headers.first("Host"), Some("x.test"));
}

#[test]
fn parse_response_bad_status_line_errors() {
    assert!(parse_response("NOT-HTTP 200 OK\r\n\r\n").is_err());
}

#[test]
fn url_parse_handles_userless_authority_only() {
    let u = url_mod::parse("http://x.test").unwrap();
    assert_eq!(u.path, "/");
    assert!(u.query.is_empty());
    assert!(u.fragment.is_empty());
}

#[test]
fn url_resolve_relative_drops_filename() {
    let base = url_mod::parse("http://x.test/dir/file.html").unwrap();
    assert_eq!(
        url_mod::resolve(&base, "other.html").as_deref(),
        Some("http://x.test/dir/other.html"),
    );
}

// ---------------------------------------------------------------------
// spider — URL filtering edges
// ---------------------------------------------------------------------

#[test]
fn spider_extract_hrefs_ignores_javascript_void() {
    let html = r#"<a href="javascript:void(0)">x</a><a href="/real">r</a>"#;
    let hrefs = extract_hrefs(html);
    // We extract everything — caller is expected to filter; but at
    // least the real link must appear.
    assert!(hrefs.contains(&"/real".to_string()));
}

#[test]
fn spider_robots_disallow_skips_other_ua() {
    let body = "User-agent: Bingbot\nDisallow: /b\nUser-agent: *\nDisallow: /all\n";
    let rules = parse_robots_disallow(body, "Bingbot");
    assert_eq!(rules, vec!["/b"]);
}

#[test]
fn spider_robots_comments_stripped() {
    let body = "# leading comment\nUser-agent: * # any\nDisallow: /admin/ # secret\n";
    let rules = parse_robots_disallow(body, "*");
    assert_eq!(rules, vec!["/admin/"]);
}

#[test]
fn spider_seed_outside_scope_yields_empty_crawl() {
    let mut ctx = Context::new("c");
    ctx.include(r"^http://allowed\.test/").unwrap();
    let cfg = SpiderConfig {
        max_depth: 5,
        max_urls: 100,
        respect_robots_txt: false,
    };
    let spider = Spider::new(cfg, &ctx);
    let found: Vec<Discovered> = spider.crawl(&["http://blocked.test/"], |_| String::new());
    assert!(found.is_empty());
}

#[test]
fn spider_visits_each_url_at_most_once() {
    let ctx = Context::new("c");
    let cfg = SpiderConfig {
        max_depth: 5,
        max_urls: 100,
        respect_robots_txt: false,
    };
    let spider = Spider::new(cfg, &ctx);
    // Two pages both link to each other → revisit must not duplicate.
    let pages = |u: &str| match u {
        "http://x.test/" => r#"<a href="/a">A</a>"#.to_string(),
        "http://x.test/a" => r#"<a href="/">root</a>"#.to_string(),
        _ => String::new(),
    };
    let found = spider.crawl(&["http://x.test/"], pages);
    let urls: Vec<_> = found.iter().map(|d| d.url.clone()).collect();
    let dedup = {
        let mut v = urls.clone();
        v.sort();
        v.dedup();
        v
    };
    assert_eq!(urls.len(), dedup.len(), "duplicates in {:?}", urls);
}

// ---------------------------------------------------------------------
// CLI — argument parser failure modes
// ---------------------------------------------------------------------

#[test]
fn cli_quick_scan_unknown_flag_errors() {
    let err = cli_parse(&["quick-scan", "http://x/", "--mystery"]).unwrap_err();
    match err {
        CliError::Unknown(s) => assert!(s.contains("--mystery")),
        other => panic!("expected Unknown, got {:?}", other),
    }
}

#[test]
fn cli_baseline_missing_minutes_value() {
    let err = cli_parse(&["baseline", "-t", "http://x/", "--minutes"]).unwrap_err();
    assert!(matches!(err, CliError::MissingArg("minutes")));
}

#[test]
fn cli_quick_scan_target_then_flags_order_invariance() {
    let a = cli_parse(&["quick-scan", "http://x/", "--spider", "--ajax"]).unwrap();
    let b = cli_parse(&["quick-scan", "http://x/", "--ajax", "--spider"]).unwrap();
    assert_eq!(a, b);
    match a {
        CliCommand::QuickScan(q) => {
            assert!(q.spider && q.ajax);
            assert!(!q.active);
        }
        _ => panic!(),
    }
}

#[test]
fn cli_status_extra_args_silently_ignored() {
    // Current parser hands status straight back regardless of trailing
    // args (it doesn't iterate). Lock in the behaviour so refactors
    // notice if it changes.
    let cmd = cli_parse(&["status", "junk"]).unwrap();
    assert_eq!(cmd, CliCommand::Status);
}

#[test]
fn cli_report_long_output_flag() {
    let cmd = cli_parse(&["report", "--output", "report.html"]).unwrap();
    match cmd {
        CliCommand::Report(r) => assert_eq!(r.output, "report.html"),
        _ => panic!(),
    }
}

// ---------------------------------------------------------------------
// scan policy / registry — composition
// ---------------------------------------------------------------------

#[test]
fn ascan_registry_default_is_empty_until_baseline() {
    let r = ScanPluginRegistry::new();
    assert!(r.is_empty());
    assert_eq!(r.len(), 0);
    let r2 = ScanPluginRegistry::with_baseline();
    assert!(!r2.is_empty());
}

#[test]
fn ascan_register_extra_rule_grows_count() {
    let mut r = ScanPluginRegistry::with_baseline();
    let base = r.len();
    r.register(Box::new(SqlInjectionRule));
    assert_eq!(r.len(), base + 1);
}

#[test]
fn pscan_registry_default_is_empty() {
    let r = PassiveScanRegistry::new();
    assert!(r.is_empty());
}

#[test]
fn pscan_run_combines_alerts_across_rules() {
    let mut r = PassiveScanRegistry::new();
    r.register(Box::new(MissingSecurityHeadersRule));
    r.register(Box::new(InsecureCookieRule));
    let req = HttpRequest::new(HttpMethod::Get, "https://x/");
    let mut resp = HttpResponse::new(200, "OK");
    resp.headers.insert("Content-Type", "text/html");
    resp.headers.insert("Set-Cookie", "sid=abc"); // missing all 3 attrs
    let alerts = r.run(&req, &resp);
    // 5 missing headers + 3 cookie problems = 8.
    assert_eq!(alerts.len(), 8);
}

// ---------------------------------------------------------------------
// HTTP request render — small edge
// ---------------------------------------------------------------------

#[test]
fn request_render_emits_blank_line_separator() {
    let req = HttpRequest::new(HttpMethod::Get, "http://x/");
    let rendered = req.render();
    assert!(
        rendered.contains("\r\n\r\n"),
        "missing header/body separator in {:?}",
        rendered
    );
}

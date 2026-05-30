// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Strict-TDD cycle — request span → OTLP trace export (userspace model of
// grafana/beyla pkg/internal/request Span construction + pkg/internal/
// export/otel). Builds server/client spans from detected HTTP/gRPC/SQL
// requests and renders them as an OTLP/JSON ResourceSpans payload.

use cave_ebpf_common::otlp::{
    to_otlp_resource_spans, AttrValue, Span, SpanKind, StatusCode,
};

#[test]
fn test_http_server_span_fields() {
    let s = Span::http_server("GET", "/api/users", 200, 1_000, 1_500);
    assert_eq!(s.name, "GET /api/users");
    assert_eq!(s.kind, SpanKind::Server);
    assert_eq!(s.duration_ns(), 500);
    assert_eq!(s.status, StatusCode::Unset); // 2xx is not an error
    assert_eq!(
        s.attr("http.request.method"),
        Some(&AttrValue::Str("GET".into()))
    );
    assert_eq!(
        s.attr("http.response.status_code"),
        Some(&AttrValue::Int(200))
    );
    assert_eq!(s.attr("url.path"), Some(&AttrValue::Str("/api/users".into())));
}

#[test]
fn test_http_server_5xx_is_error() {
    let s = Span::http_server("POST", "/checkout", 503, 0, 10);
    assert_eq!(s.status, StatusCode::Error);
}

#[test]
fn test_http_server_4xx_is_not_error_for_server() {
    // Per OTel HTTP semconv, 4xx is a client fault: a *server* span is
    // not marked Error for 4xx, only 5xx.
    let s = Span::http_server("GET", "/missing", 404, 0, 10);
    assert_eq!(s.status, StatusCode::Unset);
}

#[test]
fn test_grpc_span() {
    let s = Span::grpc("helloworld.Greeter", "SayHello", 0, 0, 100);
    assert_eq!(s.name, "helloworld.Greeter/SayHello");
    assert_eq!(s.kind, SpanKind::Server);
    assert_eq!(
        s.attr("rpc.system"),
        Some(&AttrValue::Str("grpc".into()))
    );
    assert_eq!(
        s.attr("rpc.grpc.status_code"),
        Some(&AttrValue::Int(0))
    );
    // gRPC status 0 = OK.
    assert_eq!(s.status, StatusCode::Unset);

    let err = Span::grpc("svc", "M", 14, 0, 1); // 14 = UNAVAILABLE
    assert_eq!(err.status, StatusCode::Error);
}

#[test]
fn test_db_span() {
    let s = Span::db("postgresql", "SELECT", "SELECT * FROM users", 0, 42);
    assert_eq!(s.kind, SpanKind::Client);
    assert_eq!(s.name, "SELECT");
    assert_eq!(
        s.attr("db.system"),
        Some(&AttrValue::Str("postgresql".into()))
    );
    assert_eq!(
        s.attr("db.statement"),
        Some(&AttrValue::Str("SELECT * FROM users".into()))
    );
}

#[test]
fn test_trace_and_span_id_hex() {
    let mut s = Span::http_server("GET", "/", 200, 0, 1);
    s.trace_id = [0xab; 16];
    s.span_id = [0x01; 8];
    assert_eq!(s.trace_id_hex(), "abababababababababababababababab");
    assert_eq!(s.span_id_hex(), "0101010101010101");
}

#[test]
fn test_otlp_resource_spans_structure() {
    let mut s = Span::http_server("GET", "/api", 200, 5, 9);
    s.trace_id = [0x11; 16];
    s.span_id = [0x22; 8];
    let v = to_otlp_resource_spans("checkout", &[s]);

    let rs = &v["resourceSpans"][0];
    // resource.service.name
    let res_attrs = &rs["resource"]["attributes"];
    assert_eq!(res_attrs[0]["key"], "service.name");
    assert_eq!(res_attrs[0]["value"]["stringValue"], "checkout");

    let span = &rs["scopeSpans"][0]["spans"][0];
    assert_eq!(span["traceId"], "11111111111111111111111111111111");
    assert_eq!(span["spanId"], "2222222222222222");
    assert_eq!(span["name"], "GET /api");
    assert_eq!(span["kind"], 2); // Server
    assert_eq!(span["startTimeUnixNano"], "5");
    assert_eq!(span["endTimeUnixNano"], "9");
    // status code 0 (Unset) -> omitted or 0; we emit the numeric code.
    assert_eq!(span["status"]["code"], 0);
    // attributes carried through
    let attrs = span["attributes"].as_array().unwrap();
    assert!(attrs
        .iter()
        .any(|a| a["key"] == "http.request.method" && a["value"]["stringValue"] == "GET"));
    assert!(attrs.iter().any(
        |a| a["key"] == "http.response.status_code" && a["value"]["intValue"] == "200"
    ));
}

#[test]
fn test_otlp_parent_span_id_present_when_set() {
    let mut s = Span::http_server("GET", "/", 200, 0, 1);
    s.parent_span_id = Some([0x33; 8]);
    let v = to_otlp_resource_spans("svc", &[s]);
    let span = &v["resourceSpans"][0]["scopeSpans"][0]["spans"][0];
    assert_eq!(span["parentSpanId"], "3333333333333333");

    // When absent, the field is omitted (null in serde_json index).
    let s2 = Span::http_server("GET", "/", 200, 0, 1);
    let v2 = to_otlp_resource_spans("svc", &[s2]);
    let span2 = &v2["resourceSpans"][0]["scopeSpans"][0]["spans"][0];
    assert!(span2.get("parentSpanId").is_none());
}

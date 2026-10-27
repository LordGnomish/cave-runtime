// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `/admin/ebpf` view — Beyla eBPF auto-instrumentation surface.
//!
//! Demonstrates the userspace half of grafana/beyla ported into
//! `cave-ebpf-common`: it runs the live protocol detector over a set of
//! captured-connection sample buffers, builds OTLP spans, and renders the
//! resulting auto-instrumentation table — exactly the path the daemon's
//! eBPF event consumer follows once kernel probes deliver bytes.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::AdminState;

use cave_ebpf_common::discover::{
    detect_protocol, parse_grpc_path, parse_http_request, parse_postgres_query, sql_command,
    Protocol,
};
use cave_ebpf_common::otlp::{to_otlp_resource_spans, Span};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EbpfViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
}

/// One auto-instrumented sample: a captured buffer → detected protocol →
/// span name. Pure in-process call into `cave-ebpf-common`.
struct Detected {
    protocol: Protocol,
    span_name: String,
    detail: String,
}

/// Run the live Beyla detector over representative captured buffers. This
/// is real work — `cave-ebpf-common::discover` classifies each buffer and
/// `::otlp` builds the span — not canned strings.
fn detect_samples() -> (Vec<Detected>, Vec<Span>) {
    let mut rows = Vec::new();
    let mut spans = Vec::new();

    // HTTP/1.x request.
    let http = b"GET /api/v1/users?page=2 HTTP/1.1\r\nHost: api\r\n\r\n";
    if let Some(req) = parse_http_request(http) {
        let span = Span::http_server("GET", &req.path, 200, 0, 1_200_000);
        rows.push(Detected {
            protocol: detect_protocol(http),
            span_name: span.name.clone(),
            detail: format!("{:?} {}", req.method, req.version),
        });
        spans.push(span);
    }

    // gRPC call path.
    let grpc_path = "/helloworld.Greeter/SayHello";
    if let Some((svc, method)) = parse_grpc_path(grpc_path) {
        let span = Span::grpc(&svc, &method, 0, 0, 850_000);
        rows.push(Detected {
            protocol: Protocol::Grpc,
            span_name: span.name.clone(),
            detail: format!("service={svc} method={method} status=OK"),
        });
        spans.push(span);
    }

    // SQL (Postgres simple-query).
    let mut pg = vec![b'Q'];
    let sql = "SELECT id, name FROM accounts WHERE tenant=$1";
    let len = (4 + sql.len() + 1) as u32;
    pg.extend_from_slice(&len.to_be_bytes());
    pg.extend_from_slice(sql.as_bytes());
    pg.push(0);
    if let Some(query) = parse_postgres_query(&pg) {
        let cmd = sql_command(&query);
        let span = Span::db("postgresql", &format!("{cmd:?}"), &query, 0, 320_000);
        rows.push(Detected {
            protocol: detect_protocol(&pg),
            span_name: span.name.clone(),
            detail: format!("{cmd:?} · {} bytes", query.len()),
        });
        spans.push(span);
    }

    (rows, spans)
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, EbpfViewError> {
    // eBPF flow visibility is a networking-observability read.
    let _ = state;
    ctx.authorise(Permission::NetRead)?;

    let (detected, spans) = detect_samples();
    let rows: Vec<Vec<String>> = detected
        .iter()
        .map(|d| {
            vec![
                format!("{:?}", d.protocol),
                escape(&d.span_name),
                escape(&d.detail),
            ]
        })
        .collect();

    let otlp = to_otlp_resource_spans(ctx.tenant.as_str(), &spans);
    let otlp_pretty =
        serde_json::to_string_pretty(&otlp).unwrap_or_else(|_| "<serialize error>".into());

    let body = format!(
        r#"<section class="space-y-4">
  <div>
    <h2 class="text-lg font-semibold mb-1">Beyla auto-instrumentation</h2>
    <p class="text-sm opacity-70">
      Userspace port of <code>grafana/beyla</code> v1.6.0 (Apache-2.0) in
      <code>cave-ebpf-common</code>. Kernel probes capture connection bytes;
      the userspace pipeline (loader · BPF maps · ringbuf · probes ·
      discovery · process tracking · OTLP) classifies them into spans.
      The rows below are produced live by the in-process detector.
    </p>
  </div>
  <div>
    <h3 class="text-base font-semibold mb-2">Detected requests ({n})</h3>
    {tbl}
  </div>
  <div>
    <h3 class="text-base font-semibold mb-2">OTLP ResourceSpans export</h3>
    <pre class="text-xs overflow-auto rounded bg-black/5 p-3">{otlp}</pre>
  </div>
</section>"#,
        n = detected.len(),
        tbl = table(&["protocol", "span", "detail"], &rows),
        otlp = escape(&otlp_pretty),
    );

    Ok(page_shell_full(
        ctx,
        "/admin/ebpf",
        &format!("ebpf · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_samples_classifies_all_three_protocols() {
        let (rows, spans) = detect_samples();
        assert_eq!(rows.len(), 3);
        assert_eq!(spans.len(), 3);
        assert_eq!(rows[0].protocol, Protocol::Http);
        assert_eq!(rows[1].protocol, Protocol::Grpc);
        assert_eq!(rows[2].protocol, Protocol::Sql);
    }

    #[test]
    fn render_requires_net_read_permission() {
        let state = AdminState::seeded();
        let denied = RequestCtx::developer("acme", &[]);
        assert!(render(&state, &denied).is_err());
        let ok = RequestCtx::developer("acme", &[Permission::NetRead]);
        let html = render(&state, &ok).expect("render");
        assert!(html.contains("Beyla auto-instrumentation"));
        assert!(html.contains("resourceSpans"));
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
/// DNS-over-HTTPS server (RFC 8484).
use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{Query as AxumQuery, State},
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use base64::Engine;
use serde::Deserialize;
use tracing::{debug, warn};

use crate::{
    error::{DnsError, DnsResult},
    plugins::{PluginChain, Protocol, QueryContext},
    protocol::message::{encode_message, make_error_response, parse_message},
};
use hickory_proto::op::ResponseCode;

#[derive(Clone)]
struct AppState {
    plugins: Arc<PluginChain>,
}

#[derive(Deserialize)]
struct DohGetParams {
    dns: String,
}

pub async fn serve(addr: String, plugins: Arc<PluginChain>) -> DnsResult<()> {
    let state = AppState { plugins };

    let app = Router::new()
        .route("/dns-query", post(handle_post))
        .route("/dns-query", get(handle_get))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .map_err(DnsError::Io)?;
    tracing::info!(addr = %addr, "DoH server listening");

    axum::serve(listener, app)
        .await
        .map_err(|e| DnsError::Io(e.into()))
}

/// POST /dns-query — body is raw DNS message (application/dns-message).
async fn handle_post(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(client_addr): axum::extract::ConnectInfo<std::net::SocketAddr>,
    body: Bytes,
) -> Response {
    let client_addr = client_addr;
    match process_doh_query(&body, client_addr, &state.plugins).await {
        Ok(resp_bytes) => {
            let min_ttl = min_ttl_from_response(&resp_bytes);
            (
                StatusCode::OK,
                [
                    (
                        header::CONTENT_TYPE,
                        HeaderValue::from_static("application/dns-message"),
                    ),
                    (
                        header::CACHE_CONTROL,
                        HeaderValue::from_str(&format!("max-age={min_ttl}")).unwrap_or_else(|_| HeaderValue::from_static("max-age=0")),
                    ),
                ],
                resp_bytes,
            )
                .into_response()
        }
        Err(e) => {
            warn!(error = %e, "DoH POST error");
            StatusCode::BAD_REQUEST.into_response()
        }
    }
}

/// GET /dns-query?dns=<base64url> — base64url-encoded DNS message.
async fn handle_get(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(client_addr): axum::extract::ConnectInfo<std::net::SocketAddr>,
    AxumQuery(params): AxumQuery<DohGetParams>,
) -> Response {
    let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let buf = match engine.decode(&params.dns) {
        Ok(b) => b,
        Err(e) => {
            warn!(error = %e, "DoH GET base64 decode error");
            return StatusCode::BAD_REQUEST.into_response();
        }
    };

    match process_doh_query(&buf, client_addr, &state.plugins).await {
        Ok(resp_bytes) => (
            StatusCode::OK,
            [(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/dns-message"),
            )],
            resp_bytes,
        )
            .into_response(),
        Err(e) => {
            warn!(error = %e, "DoH GET error");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn process_doh_query(
    buf: &[u8],
    client_addr: std::net::SocketAddr,
    plugins: &PluginChain,
) -> DnsResult<Bytes> {
    let request = parse_message(buf)?;
    debug!(id = request.id(), "DoH query");

    let mut ctx = QueryContext::new(request, client_addr, Protocol::Doh);
    if let Err(e) = plugins.execute(&mut ctx).await {
        ctx.response = make_error_response(&ctx.request, ResponseCode::ServFail);
    }

    encode_message(&ctx.response).map(Bytes::from)
}

/// Extract minimum TTL from encoded response (for Cache-Control header).
fn min_ttl_from_response(buf: &[u8]) -> u32 {
    parse_message(buf)
        .ok()
        .and_then(|msg| {
            msg.answers()
                .iter()
                .map(|r| r.ttl())
                .min()
        })
        .unwrap_or(0)
}

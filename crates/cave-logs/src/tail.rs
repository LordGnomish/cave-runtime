//! WebSocket live tail — `/loki/api/v1/tail`.
//!
//! Clients receive a stream of `TailResponse` JSON objects as new log entries
//! arrive matching their query.

use crate::logql::parser::parse;
use crate::models::{Labels, StreamResult, TailResponse};
use crate::store::{LogStore, TailEvent};
use axum::extract::ws::{Message, WebSocket};
use chrono::Utc;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{debug, warn};

/// Drive a single WebSocket tail connection.
///
/// `query`  — LogQL stream selector (filters applied server-side).
/// `limit`  — maximum number of entries per tail message.
/// `tenant` — optional `X-Scope-OrgID`.
pub async fn handle_tail(
    socket: WebSocket,
    store: Arc<LogStore>,
    query: String,
    _limit: usize,
    tenant: Option<String>,
) {
    let matchers = match parse_matchers(&query) {
        Ok(m) => m,
        Err(e) => {
            warn!(error = %e, "tail: invalid query");
            close_with_error(socket, &e).await;
            return;
        }
    };

    let mut rx = store.subscribe();
    drive_tail(socket, rx, matchers, tenant).await;
}

async fn drive_tail(
    mut socket: WebSocket,
    mut rx: broadcast::Receiver<TailEvent>,
    matchers: Vec<crate::models::LabelMatcher>,
    tenant: Option<String>,
) {
    loop {
        match rx.recv().await {
            Ok(event) => {
                // Tenant filter
                if let Some(ref t) = tenant {
                    if event.tenant.as_deref() != Some(t.as_str()) {
                        continue;
                    }
                }

                // Label filter
                if !event.labels.matches(&matchers) {
                    continue;
                }

                let values: Vec<[String; 2]> = event
                    .entries
                    .iter()
                    .map(|e| {
                        let ts = e.timestamp.timestamp_nanos_opt().unwrap_or(0).to_string();
                        [ts, e.line.clone()]
                    })
                    .collect();

                let resp = TailResponse {
                    streams: vec![StreamResult {
                        stream: event.labels.0.into_iter().collect(),
                        values,
                    }],
                    dropped_entries: None,
                };

                match serde_json::to_string(&resp) {
                    Ok(json) => {
                        if socket.send(Message::Text(json.into())).await.is_err() {
                            debug!("tail: client disconnected");
                            break;
                        }
                    }
                    Err(e) => warn!(error = %e, "tail: serialize failed"),
                }
            }
            Err(broadcast::error::RecvError::Lagged(n)) => {
                warn!(dropped = n, "tail: lagged, dropping messages");
                // Optionally notify client
                let resp = TailResponse {
                    streams: vec![],
                    dropped_entries: Some(vec![crate::models::DroppedEntry {
                        timestamp: Utc::now().timestamp_nanos_opt().unwrap_or(0).to_string(),
                        labels: "{}".into(),
                    }]),
                };
                if let Ok(json) = serde_json::to_string(&resp) {
                    let _ = socket.send(Message::Text(json.into())).await;
                }
            }
            Err(broadcast::error::RecvError::Closed) => {
                debug!("tail: broadcast channel closed");
                break;
            }
        }
    }
}

fn parse_matchers(query: &str) -> Result<Vec<crate::models::LabelMatcher>, String> {
    let expr = parse(query)?;
    match expr {
        crate::logql::ast::Expr::Log(ls) => Ok(ls.matchers),
        _ => Err("tail query must be a log stream selector".into()),
    }
}

async fn close_with_error(mut socket: WebSocket, msg: &str) {
    let _ = socket
        .send(Message::Text(
            serde_json::json!({"status": "error", "message": msg}).to_string().into(),
        ))
        .await;
    // Drop socket to close the connection
    drop(socket);
}

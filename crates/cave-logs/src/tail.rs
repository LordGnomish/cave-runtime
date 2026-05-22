// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! WebSocket log tail — /loki/api/v1/tail
//!
//! Subscribers connect via WebSocket and receive a stream of `TailResponse`
//! JSON messages matching their LogQL filter query.

use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::warn;

use crate::logql::ast::{PipelineStage, Query};
use crate::logql::eval::{apply_pipeline, labels_match};
use crate::logql::parser::Parser;
use crate::models::{TailEntry, TailEvent, TailResponse, TailStream};
use crate::store::LogStore;

/// Handle one WebSocket tail connection.
pub async fn handle_tail(ws: WebSocket, logql_query: String, tenant: String, store: Arc<LogStore>) {
    let (mut sender, mut receiver) = ws.split();

    // Parse the LogQL query — we only use it for filtering.
    let (selector, pipeline) = match Parser::parse_query(&logql_query) {
        Ok(Query::Log(lq)) => (lq.selector, lq.pipeline),
        Ok(_) => {
            let _ = sender
                .send(Message::Text(
                    r#"{"error":"tail requires a log query, not a metric query"}"#.into(),
                ))
                .await;
            return;
        }
        Err(e) => {
            let _ = sender
                .send(Message::Text(
                    format!(r#"{{"error":"parse error: {}"}}"#, e).into(),
                ))
                .await;
            return;
        }
    };

    let mut rx: broadcast::Receiver<TailEvent> = store.subscribe();

    loop {
        tokio::select! {
            // New log event from the broadcast channel.
            event = rx.recv() => {
                match event {
                    Ok(ev) => {
                        // Filter by tenant.
                        if ev.tenant != tenant {
                            continue;
                        }
                        // Filter by stream selector.
                        if !labels_match(&ev.stream_labels, &selector) {
                            continue;
                        }
                        // Apply pipeline filter stages.
                        if let Some(_processed) = apply_pipeline(&ev.entry, &ev.stream_labels, &pipeline) {
                            let resp = TailResponse {
                                streams: vec![TailStream {
                                    stream: ev.stream_labels.0.clone(),
                                    values: vec![(ev.entry.ts.to_string(), ev.entry.line.clone())],
                                }],
                                dropped_entries: vec![],
                            };
                            if let Ok(json) = serde_json::to_string(&resp) {
                                if sender.send(Message::Text(json.into())).await.is_err() {
                                    break; // client disconnected
                                }
                            }
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!("tail: broadcast channel lagged by {} messages for tenant {}", n, tenant);
                        // Notify client of dropped entries.
                        let resp = TailResponse {
                            streams: vec![],
                            dropped_entries: vec![], // we don't have details on which entries were dropped
                        };
                        let json = serde_json::to_string(&resp).unwrap_or_default();
                        if sender.send(Message::Text(json.into())).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }

            // Client sends a message (ping/close).
            msg = receiver.next() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(Message::Ping(data))) => {
                        let _ = sender.send(Message::Pong(data)).await;
                    }
                    _ => {}
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    // WebSocket tests require a running server; we test the filter logic only.
    use crate::logql::ast::{LabelMatcher, LineFilter, MatchOp, PipelineStage, StreamSelector};
    use crate::logql::eval::{apply_pipeline, labels_match};
    use crate::models::{Labels, LogEntry};
    use std::collections::HashMap;

    #[test]
    fn tail_filter_matches() {
        let labels = Labels::new(HashMap::from([("app".into(), "nginx".into())]));
        let selector = StreamSelector {
            matchers: vec![LabelMatcher {
                name: "app".into(),
                op: MatchOp::Eq,
                value: "nginx".into(),
            }],
        };
        assert!(labels_match(&labels, &selector));

        let entry = LogEntry::new(0, "error: 500");
        let pipeline = vec![PipelineStage::LineFilter(LineFilter::Contains(
            "error".into(),
        ))];
        assert!(apply_pipeline(&entry, &labels, &pipeline).is_some());
    }

    #[test]
    fn tail_filter_no_match() {
        let labels = Labels::new(HashMap::from([("app".into(), "postgres".into())]));
        let selector = StreamSelector {
            matchers: vec![LabelMatcher {
                name: "app".into(),
                op: MatchOp::Eq,
                value: "nginx".into(),
            }],
        };
        assert!(!labels_match(&labels, &selector));
    }
}

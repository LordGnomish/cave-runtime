// SPDX-License-Identifier: AGPL-3.0-or-later
//! etcd Watch service — bidirectional streaming watch API.

use std::sync::Arc;

use async_stream::try_stream;
use futures_core::Stream;
use tokio_stream::StreamExt;
use tonic::{Request, Response, Status, Streaming};

use crate::engine::{mvcc::WatchEventType, StorageEngine};

use super::proto::{
    etcdserverpb::{
        watch_request::RequestUnion,
        watch_server::Watch,
        watch_create_request::FilterType,
        WatchCreateRequest, WatchRequest, WatchResponse, ResponseHeader,
    },
    mvccpb,
};

pub struct WatchServer {
    engine: Arc<StorageEngine>,
}

impl WatchServer {
    pub fn new(engine: Arc<StorageEngine>) -> Self {
        Self { engine }
    }
}

fn kv_to_proto(kv: crate::engine::KeyValue) -> mvccpb::KeyValue {
    mvccpb::KeyValue {
        key: kv.key,
        value: kv.value,
        create_revision: kv.create_revision,
        mod_revision: kv.mod_revision,
        version: kv.version,
        lease: kv.lease,
    }
}

fn current_header_values(engine: &StorageEngine) -> (ResponseHeader, i64) {
    let mvcc = engine.mvcc.read();
    let header = ResponseHeader {
        cluster_id: 1,
        member_id: 1,
        revision: mvcc.current_revision(),
        raft_term: 1,
    };
    let compact_rev = mvcc.compacted_revision();
    (header, compact_rev)
}

fn make_header(engine: &StorageEngine) -> ResponseHeader {
    let mvcc = engine.mvcc.read();
    ResponseHeader {
        cluster_id: 1,
        member_id: 1,
        revision: mvcc.current_revision(),
        raft_term: 1,
    }
}

#[tonic::async_trait]
impl Watch for WatchServer {
    type WatchStream = std::pin::Pin<Box<dyn Stream<Item = Result<WatchResponse, Status>> + Send>>;

    async fn watch(
        &self,
        req: Request<Streaming<WatchRequest>>,
    ) -> Result<Response<Self::WatchStream>, Status> {
        let engine = Arc::clone(&self.engine);
        let mut inbound = req.into_inner();

        let stream = try_stream! {
            // Obtain receiver before entering the loop; guard is dropped immediately.
            let mut rx = { engine.mvcc.read().subscribe() };
            let mut active_watches: std::collections::HashSet<i64> = std::collections::HashSet::new();

            loop {
                tokio::select! {
                    msg = inbound.next() => {
                        match msg {
                            None => break,
                            Some(Err(e)) => {
                                tracing::warn!("watch stream error: {e}");
                                break;
                            }
                            Some(Ok(req)) => {
                                match req.request_union {
                                    Some(RequestUnion::CreateRequest(cr)) => {
                                        let watch_id = handle_create(&engine, cr, &mut active_watches);
                                        // Extract all values from lock before yield.
                                        let (header, compact_revision) = current_header_values(&engine);
                                        yield WatchResponse {
                                            header: Some(header),
                                            watch_id,
                                            created: true,
                                            canceled: false,
                                            compact_revision,
                                            cancel_reason: String::new(),
                                            fragment: false,
                                            events: vec![],
                                        };
                                    }
                                    Some(RequestUnion::CancelRequest(cr)) => {
                                        let watch_id = cr.watch_id;
                                        engine.mvcc.write().watch_cancel(watch_id);
                                        active_watches.remove(&watch_id);
                                        let header = make_header(&engine);
                                        yield WatchResponse {
                                            header: Some(header),
                                            watch_id,
                                            created: false,
                                            canceled: true,
                                            compact_revision: 0,
                                            cancel_reason: "watch canceled".to_string(),
                                            fragment: false,
                                            events: vec![],
                                        };
                                    }
                                    Some(RequestUnion::ProgressRequest(_)) => {
                                        let header = make_header(&engine);
                                        yield WatchResponse {
                                            header: Some(header),
                                            watch_id: 0,
                                            created: false,
                                            canceled: false,
                                            compact_revision: 0,
                                            cancel_reason: String::new(),
                                            fragment: false,
                                            events: vec![],
                                        };
                                    }
                                    None => {}
                                }
                            }
                        }
                    }
                    event = rx.recv() => {
                        match event {
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                                tracing::warn!("watch receiver lagged {n} events");
                            }
                            Err(_) => break,
                            Ok(ev) => {
                                if !active_watches.contains(&ev.watch_id) {
                                    continue;
                                }
                                let event_type = match ev.event_type {
                                    WatchEventType::Put => mvccpb::event::EventType::Put as i32,
                                    WatchEventType::Delete => mvccpb::event::EventType::Delete as i32,
                                };
                                let proto_event = mvccpb::Event {
                                    r#type: event_type,
                                    kv: Some(kv_to_proto(ev.kv)),
                                    prev_kv: ev.prev_kv.map(kv_to_proto),
                                };
                                let header = make_header(&engine);
                                yield WatchResponse {
                                    header: Some(header),
                                    watch_id: ev.watch_id,
                                    created: false,
                                    canceled: false,
                                    compact_revision: 0,
                                    cancel_reason: String::new(),
                                    fragment: false,
                                    events: vec![proto_event],
                                };
                            }
                        }
                    }
                }
            }
        };

        Ok(Response::new(Box::pin(stream)))
    }
}

fn handle_create(
    engine: &StorageEngine,
    cr: WatchCreateRequest,
    active_watches: &mut std::collections::HashSet<i64>,
) -> i64 {
    let no_put = cr.filters.contains(&(FilterType::Noput as i32));
    let no_delete = cr.filters.contains(&(FilterType::Nodelete as i32));
    let watch_id = engine.mvcc.write().watch_create(
        cr.key,
        cr.range_end,
        cr.start_revision,
        cr.progress_notify,
        cr.prev_kv,
        no_put,
        no_delete,
        cr.watch_id,
    );
    active_watches.insert(watch_id);
    watch_id
}

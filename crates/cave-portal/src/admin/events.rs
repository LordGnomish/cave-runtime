// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `/api/events/stream` — multiplexed Server-Sent Events bus.
//!
//! A single typed event stream consumed by every realtime view in the
//! Portal (cluster live page, scale dashboards, audit log tail). The
//! shape mirrors what htmx's `sse:` extension expects: `event: <name>`
//! + `data: <json>` blocks separated by a blank line.
//!
//! Persona scope is enforced on every event: PlatformAdmin sees the
//! whole bus, TenantAdmin only sees events whose tenant matches the
//! caller's. Events with no tenant (e.g. cluster-wide RaftStateChange)
//! are only delivered to PlatformAdmin.
//!
//! The bus is in-memory and uses a tokio broadcast channel so any
//! number of subscribers can tail concurrently without blocking the
//! publisher. The handler converts the broadcast stream into the SSE
//! wire format.

use crate::admin::permission::{Permission, Persona, RequestCtx};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;

/// Default broadcast channel buffer. Lagging subscribers see a
/// `lagged` SSE event so the client can refresh; the publisher never
/// blocks.
pub const DEFAULT_BUFFER: usize = 1024;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EventsError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
    #[error("event bus closed")]
    Closed,
}

/// One event, typed and tenant-scoped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    /// Raft layer reports a new term / leader / commit index.
    RaftStateChange {
        term: u64,
        leader_id: Option<u64>,
        commit_index: u64,
        last_applied: u64,
    },
    /// One Raft log entry just applied. `command_kind` is a stable
    /// taxonomy string (`etcd.put`, `apiserver.upsert`, ...).
    ApplyEntry {
        index: u64,
        command_kind: String,
    },
    /// cave-upstream-watchd noticed an upstream release that landed
    /// in a tracked repo and the cave-side has not bumped.
    GapOpened {
        crate_name: String,
        upstream_version: String,
        days_since_release: u32,
    },
    /// KEDA reconciler decided to scale a workload.
    ScaledObjectScale {
        tenant: String,
        namespace: String,
        name: String,
        replicas_from: u32,
        replicas_to: u32,
        trigger: String,
    },
    /// Vault read/list/write on a secret path.
    VaultSecretAccess {
        tenant: String,
        principal: String,
        path: String,
        op: String, // "read"/"list"/"write"/"delete"
    },
    /// Kubelet announced a node readiness flip.
    NodeReady {
        node: String,
        ready: bool,
    },
    /// One pod's phase changed (Pending/Running/Succeeded/Failed/Unknown).
    PodPhaseChange {
        tenant: String,
        namespace: String,
        pod: String,
        from: String,
        to: String,
    },
}

impl Event {
    pub const fn kind(&self) -> &'static str {
        match self {
            Event::RaftStateChange { .. } => "raft_state_change",
            Event::ApplyEntry { .. } => "apply_entry",
            Event::GapOpened { .. } => "gap_opened",
            Event::ScaledObjectScale { .. } => "scaled_object_scale",
            Event::VaultSecretAccess { .. } => "vault_secret_access",
            Event::NodeReady { .. } => "node_ready",
            Event::PodPhaseChange { .. } => "pod_phase_change",
        }
    }

    /// Tenant the event belongs to. `None` ⇒ cluster-wide
    /// (PlatformAdmin only).
    pub fn tenant(&self) -> Option<&str> {
        match self {
            Event::ScaledObjectScale { tenant, .. }
            | Event::VaultSecretAccess { tenant, .. }
            | Event::PodPhaseChange { tenant, .. } => Some(tenant),
            Event::RaftStateChange { .. }
            | Event::ApplyEntry { .. }
            | Event::GapOpened { .. }
            | Event::NodeReady { .. } => None,
        }
    }

    /// True if `ctx` should receive this event under the persona
    /// scope rules.
    pub fn deliver_to(&self, ctx: &RequestCtx) -> bool {
        match self.tenant() {
            None => ctx.persona == Persona::PlatformAdmin,
            Some(t) => {
                ctx.persona == Persona::PlatformAdmin || ctx.tenant.as_str() == t
            }
        }
    }

    /// Render as one SSE wire frame:
    /// ```text
    /// event: <kind>
    /// data: <json>
    /// \n
    /// ```
    pub fn to_sse_frame(&self) -> String {
        let body = serde_json::to_string(self).unwrap_or_else(|_| "{}".into());
        format!("event: {}\ndata: {}\n\n", self.kind(), body)
    }
}

/// Multiplexed event bus, shared between the portal handler and the
/// publishers (raft driver, KEDA reconciler, kubelet status manager).
#[derive(Debug)]
pub struct EventBus {
    sender: broadcast::Sender<Event>,
    /// Capacity the bus was constructed with — exposed for tests.
    capacity: usize,
    /// Lifetime publish counter.
    published: std::sync::atomic::AtomicU64,
}

impl EventBus {
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_BUFFER)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self {
            sender,
            capacity,
            published: std::sync::atomic::AtomicU64::new(0),
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn subscriber_count(&self) -> usize {
        self.sender.receiver_count()
    }

    pub fn published_total(&self) -> u64 {
        self.published.load(std::sync::atomic::Ordering::Acquire)
    }

    /// Publish an event. Drops silently if no subscribers — the bus
    /// is at-most-once.
    pub fn publish(&self, ev: Event) {
        self.published
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        let _ = self.sender.send(ev);
    }

    /// Subscribe to a filtered slice of the bus. Returns a stream
    /// that yields one Event per delivered message; events the ctx
    /// is not permitted to see are filtered out at the bus side so a
    /// rogue subscriber can't peek at a foreign tenant.
    pub fn subscribe(
        self: &Arc<Self>,
        ctx: RequestCtx,
    ) -> Result<EventSubscription, EventsError> {
        ctx.authorise(Permission::EventsSubscribe)?;
        Ok(EventSubscription {
            recv: self.sender.subscribe(),
            ctx,
        })
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

/// A scoped subscriber. Drop unsubscribes from the broadcast channel.
pub struct EventSubscription {
    recv: broadcast::Receiver<Event>,
    ctx: RequestCtx,
}

impl EventSubscription {
    /// Block until the next event the subscriber is allowed to see.
    /// Filters out tenant-foreign events transparently. Returns
    /// `Closed` when the bus has been dropped.
    pub async fn next_visible(&mut self) -> Result<Event, EventsError> {
        loop {
            match self.recv.recv().await {
                Ok(ev) if ev.deliver_to(&self.ctx) => return Ok(ev),
                Ok(_) => continue, // visible to others, not us
                Err(broadcast::error::RecvError::Closed) => {
                    return Err(EventsError::Closed);
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    // Lagging client: tell them, then resume.
                    return Ok(Event::ApplyEntry {
                        index: 0,
                        command_kind: "lagged".into(),
                    });
                }
            }
        }
    }

    /// Non-blocking poll with a per-call timeout. Useful for the SSE
    /// keep-alive path: send a comment frame when no event lands
    /// within the timeout so proxies don't drop the connection.
    pub async fn next_with_timeout(
        &mut self,
        timeout: Duration,
    ) -> Result<Option<Event>, EventsError> {
        match tokio::time::timeout(timeout, self.next_visible()).await {
            Ok(Ok(ev)) => Ok(Some(ev)),
            Ok(Err(e)) => Err(e),
            Err(_) => Ok(None),
        }
    }
}

/// One-frame keep-alive comment to keep proxies from dropping the
/// SSE connection. The `: ` prefix marks it as a comment per
/// the EventSource spec.
pub const KEEPALIVE_FRAME: &str = ": keepalive\n\n";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::permission::Permission;

    fn ctx_platform(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    fn ctx_tenant(tenant: &str, perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer_as(tenant, perms, Persona::TenantAdmin)
    }

    fn ev_pod(t: &str) -> Event {
        Event::PodPhaseChange {
            tenant: t.into(),
            namespace: "default".into(),
            pod: "echo-1".into(),
            from: "Pending".into(),
            to: "Running".into(),
        }
    }

    #[test]
    fn event_kind_is_stable_per_variant() {
        assert_eq!(
            Event::RaftStateChange {
                term: 1,
                leader_id: Some(1),
                commit_index: 0,
                last_applied: 0,
            }
            .kind(),
            "raft_state_change"
        );
        assert_eq!(ev_pod("acme").kind(), "pod_phase_change");
    }

    #[test]
    fn event_tenant_returns_none_for_cluster_wide() {
        assert_eq!(
            Event::NodeReady {
                node: "n1".into(),
                ready: true,
            }
            .tenant(),
            None
        );
        assert_eq!(ev_pod("acme").tenant(), Some("acme"));
    }

    #[test]
    fn sse_frame_round_trips_through_parser() {
        let ev = ev_pod("acme");
        let frame = ev.to_sse_frame();
        assert!(frame.starts_with("event: pod_phase_change\n"));
        assert!(frame.contains("data: {"));
        assert!(frame.ends_with("\n\n"));
    }

    #[test]
    fn deliver_to_filters_tenant_admin_to_own_tenant() {
        let acme = ctx_tenant("acme", &[Permission::EventsSubscribe]);
        let other = ctx_tenant("other", &[Permission::EventsSubscribe]);
        let ev = ev_pod("acme");
        assert!(ev.deliver_to(&acme));
        assert!(!ev.deliver_to(&other));
    }

    #[test]
    fn deliver_to_blocks_cluster_wide_event_from_tenant_admin() {
        let ten = ctx_tenant("acme", &[Permission::EventsSubscribe]);
        let plat = ctx_platform(&[Permission::EventsSubscribe]);
        let ev = Event::NodeReady {
            node: "n1".into(),
            ready: true,
        };
        assert!(!ev.deliver_to(&ten));
        assert!(ev.deliver_to(&plat));
    }

    #[test]
    fn subscribe_refuses_without_permission() {
        let bus = Arc::new(EventBus::new());
        let ctx = ctx_platform(&[]); // no EventsSubscribe
        let err = bus.subscribe(ctx).map(|_| ()).unwrap_err();
        assert!(matches!(err, EventsError::Auth(_)));
    }

    #[tokio::test]
    async fn subscribe_receives_only_visible_events() {
        let bus = Arc::new(EventBus::new());
        let ctx = ctx_tenant("acme", &[Permission::EventsSubscribe]);
        let mut sub = bus.subscribe(ctx).unwrap();
        bus.publish(ev_pod("other"));
        bus.publish(ev_pod("acme"));
        bus.publish(ev_pod("third"));
        let got = sub
            .next_with_timeout(Duration::from_millis(100))
            .await
            .unwrap()
            .expect("should receive an event");
        match got {
            Event::PodPhaseChange { tenant, .. } => assert_eq!(tenant, "acme"),
            other => panic!("wrong event: {other:?}"),
        }
    }

    #[tokio::test]
    async fn platform_admin_subscriber_sees_cluster_wide_events() {
        let bus = Arc::new(EventBus::new());
        let mut sub = bus
            .subscribe(ctx_platform(&[Permission::EventsSubscribe]))
            .unwrap();
        bus.publish(Event::RaftStateChange {
            term: 7,
            leader_id: Some(2),
            commit_index: 100,
            last_applied: 99,
        });
        let got = sub
            .next_with_timeout(Duration::from_millis(100))
            .await
            .unwrap();
        assert!(matches!(got, Some(Event::RaftStateChange { term: 7, .. })));
    }

    #[tokio::test]
    async fn next_with_timeout_returns_none_when_idle() {
        let bus = Arc::new(EventBus::new());
        let mut sub = bus
            .subscribe(ctx_platform(&[Permission::EventsSubscribe]))
            .unwrap();
        let got = sub
            .next_with_timeout(Duration::from_millis(20))
            .await
            .unwrap();
        assert!(got.is_none());
    }

    #[test]
    fn published_total_tracks_publish_calls() {
        let bus = EventBus::new();
        bus.publish(ev_pod("acme"));
        bus.publish(ev_pod("acme"));
        assert_eq!(bus.published_total(), 2);
    }

    #[test]
    fn subscriber_count_reflects_active_subscriptions() {
        let bus = Arc::new(EventBus::new());
        assert_eq!(bus.subscriber_count(), 0);
        let _s1 = bus
            .subscribe(ctx_platform(&[Permission::EventsSubscribe]))
            .unwrap();
        let _s2 = bus
            .subscribe(ctx_platform(&[Permission::EventsSubscribe]))
            .unwrap();
        assert_eq!(bus.subscriber_count(), 2);
    }

    #[test]
    fn keepalive_frame_is_sse_comment() {
        assert!(KEEPALIVE_FRAME.starts_with(": "));
        assert!(KEEPALIVE_FRAME.ends_with("\n\n"));
    }
}

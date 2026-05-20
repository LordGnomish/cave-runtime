// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! OOM event watcher — cgroup-v2 memory.events surfacing.
//!
//! Mirrors containerd's `pkg/oom/` watcher which feeds the kubelet's
//! eviction manager. Upstream containerd publishes a single `OomEvent`
//! per container kill with `oom_score_adj`, the killer PID, and the
//! kill timestamp; the kubelet consumes that event to update
//! ImageGC / eviction signals (KEP-2453 — node-level OOM accounting).
//!
//! Architecture:
//!
//! ```text
//!  cgroup v2 memory.events  ─►  LinuxCgroupOomSource ─┐
//!  in-process channel       ─►  InMemoryOomSource    ─┤
//!                                                    ▼
//!                                          OomWatcher (consumer)
//!                                                    │
//!                                                    ▼
//!                                          OomEventBus (broadcast)
//!                                                    │
//!                                       ┌────────────┼────────────┐
//!                                       ▼            ▼            ▼
//!                                   kubelet     eviction.rs     audit log
//! ```
//!
//! The `OomSource` trait abstracts the kernel notification channel so
//! tests can drive deterministic event streams via `InMemoryOomSource`
//! without touching `/sys/fs/cgroup/...`.
//!
//! Upstream: containerd `pkg/oom/watcher.go`, `pkg/oom/v2/epoll.go`.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

/// A single OOM-kill event published by the kernel for a container's
/// cgroup. `oom_score_adj` is the kernel's `/proc/<pid>/oom_score_adj`
/// reading at the moment of the kill (a signed integer in
/// `[-1000, 1000]`; lower values are more reluctant to be killed).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OomEvent {
    /// Container id matching `RuntimeService.ContainerStatus.id`.
    pub container_id: String,
    /// PID inside the container's pid namespace that was killed.
    pub pid: u32,
    /// Kernel `oom_score_adj` at the moment of the kill.
    pub oom_score_adj: i32,
    /// Wall-clock timestamp of the kill.
    pub killed_at: DateTime<Utc>,
}

impl OomEvent {
    /// Convenience constructor used by sources and tests.
    pub fn new(container_id: impl Into<String>, pid: u32, oom_score_adj: i32) -> Self {
        Self {
            container_id: container_id.into(),
            pid,
            oom_score_adj,
            killed_at: Utc::now(),
        }
    }

    /// Container status helper: an ExitCode of 137 (= 128 + SIGKILL) and
    /// a reason field of `OOMKilled` are the two signals containerd
    /// uses to decide whether to publish an `OomEvent`. This mirrors
    /// containerd's `pkg/cri/server/container_status.go::isOOMKilled`.
    pub fn from_exit(
        container_id: impl Into<String>,
        pid: u32,
        oom_score_adj: i32,
        exit_code: i32,
        reason: &str,
    ) -> Option<Self> {
        if exit_code == 137 && reason.eq_ignore_ascii_case("OOMKilled") {
            Some(Self::new(container_id, pid, oom_score_adj))
        } else {
            None
        }
    }
}

/// Abstract source of OOM events — production wires this to
/// cgroup-v2 `memory.events` via `inotify`, tests use an in-memory
/// queue.
#[async_trait]
pub trait OomSource: Send + Sync {
    /// Block until the next OOM event arrives. Returns `None` if the
    /// source is closed.
    async fn next(&self) -> Option<OomEvent>;
}

/// In-memory `OomSource` for tests + dev. Events are pre-seeded with
/// [`Self::push`] and drained FIFO.
pub struct InMemoryOomSource {
    queue: Mutex<VecDeque<OomEvent>>,
    notify: tokio::sync::Notify,
}

impl InMemoryOomSource {
    pub fn new() -> Self {
        Self {
            queue: Mutex::new(VecDeque::new()),
            notify: tokio::sync::Notify::new(),
        }
    }

    /// Enqueue an event; wakes one waiter on `next()`.
    pub fn push(&self, event: OomEvent) {
        self.queue.lock().expect("queue lock").push_back(event);
        self.notify.notify_one();
    }

    /// Pending event count (snapshot — racy under concurrent pushers).
    pub fn pending(&self) -> usize {
        self.queue.lock().expect("queue lock").len()
    }
}

impl Default for InMemoryOomSource {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl OomSource for InMemoryOomSource {
    async fn next(&self) -> Option<OomEvent> {
        loop {
            if let Some(e) = self.queue.lock().expect("queue lock").pop_front() {
                return Some(e);
            }
            self.notify.notified().await;
        }
    }
}

/// Linux cgroup-v2 OOM source — stubbed file-system path resolver.
///
/// In production this would set up an `inotify` watch on the
/// `memory.events` file under `<cgroup_root>/<container>/memory.events`
/// (per-cgroup `oom_kill` counter). The trait-based design above means
/// the file-watcher binding can be developed and tested behind
/// `InMemoryOomSource` first and slot into `OomWatcher::new` without
/// touching the consumer code.
pub struct LinuxCgroupOomSource {
    cgroup_root: std::path::PathBuf,
}

impl LinuxCgroupOomSource {
    pub fn new(cgroup_root: impl Into<std::path::PathBuf>) -> Self {
        Self {
            cgroup_root: cgroup_root.into(),
        }
    }

    /// Resolve the memory.events path for a given container id under
    /// the cgroup root. The layout mirrors systemd-cgroup's
    /// `kubepods.slice/.../<container>.scope/memory.events`.
    pub fn memory_events_path(&self, container_id: &str) -> std::path::PathBuf {
        self.cgroup_root.join(container_id).join("memory.events")
    }

    /// Root the watcher was constructed with.
    pub fn cgroup_root(&self) -> &std::path::Path {
        &self.cgroup_root
    }
}

#[async_trait]
impl OomSource for LinuxCgroupOomSource {
    async fn next(&self) -> Option<OomEvent> {
        // Production wiring point: inotify on memory.events. The
        // platform binding is intentionally not implemented in this
        // module so cave-cri stays single-target portable; the
        // trait-based design lets the kubelet-side consumer be tested
        // deterministically. Returning `None` here means "source is
        // closed" — `OomWatcher::run` will exit cleanly.
        None
    }
}

/// Broadcast fan-out bus for OOM events. Subscribers (kubelet,
/// eviction manager, audit log) each receive every event published
/// after they subscribed.
///
/// Built on `tokio::sync::broadcast`. Slow subscribers see
/// `RecvError::Lagged` rather than blocking the publisher.
#[derive(Clone)]
pub struct OomEventBus {
    tx: broadcast::Sender<OomEvent>,
}

impl OomEventBus {
    /// New bus with the given subscriber-side ring-buffer capacity.
    pub fn new(capacity: usize) -> Self {
        let (tx, _rx) = broadcast::channel(capacity.max(1));
        Self { tx }
    }

    /// Publish an event to every current subscriber. Returns the
    /// receiver count (0 if no subscribers — event is dropped).
    pub fn publish(&self, event: OomEvent) -> usize {
        self.tx.send(event).unwrap_or(0)
    }

    /// New subscriber receiver.
    pub fn subscribe(&self) -> broadcast::Receiver<OomEvent> {
        self.tx.subscribe()
    }

    /// Current subscriber count.
    pub fn subscriber_count(&self) -> usize {
        self.tx.receiver_count()
    }
}

impl Default for OomEventBus {
    fn default() -> Self {
        Self::new(64)
    }
}

/// The watcher loop — pulls from an `OomSource` and republishes onto
/// an `OomEventBus`. The kubelet, eviction manager, and audit log all
/// subscribe to the same bus.
pub struct OomWatcher {
    source: Arc<dyn OomSource>,
    bus: OomEventBus,
}

impl OomWatcher {
    pub fn new(source: Arc<dyn OomSource>, bus: OomEventBus) -> Self {
        Self { source, bus }
    }

    /// Read one event from the source and broadcast it. Returns
    /// `false` when the source closes.
    pub async fn tick(&self) -> bool {
        match self.source.next().await {
            Some(event) => {
                self.bus.publish(event);
                true
            }
            None => false,
        }
    }

    /// Run forever (until the source closes). Spawn this on a tokio
    /// task in `cave-runtime::serve`.
    pub async fn run(&self) {
        while self.tick().await {}
    }

    /// Bus handle for callers that need to subscribe directly.
    pub fn bus(&self) -> &OomEventBus {
        &self.bus
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oom_event_new_sets_fields() {
        let e = OomEvent::new("c-abc", 4242, -500);
        assert_eq!(e.container_id, "c-abc");
        assert_eq!(e.pid, 4242);
        assert_eq!(e.oom_score_adj, -500);
    }

    #[test]
    fn oom_event_from_exit_emits_on_137_oomkilled() {
        let e = OomEvent::from_exit("c-1", 1, 100, 137, "OOMKilled");
        assert!(e.is_some());
        let e = e.unwrap();
        assert_eq!(e.container_id, "c-1");
        assert_eq!(e.pid, 1);
        assert_eq!(e.oom_score_adj, 100);
    }

    #[test]
    fn oom_event_from_exit_case_insensitive_reason() {
        assert!(OomEvent::from_exit("c", 1, 0, 137, "oomkilled").is_some());
        assert!(OomEvent::from_exit("c", 1, 0, 137, "OomKilled").is_some());
    }

    #[test]
    fn oom_event_from_exit_rejects_non_oom_exits() {
        // Exit code 137 but wrong reason (e.g. operator SIGKILL).
        assert!(OomEvent::from_exit("c", 1, 0, 137, "Error").is_none());
        // OOMKilled reason but wrong exit code (impossible kernel-side
        // but the helper is conservative).
        assert!(OomEvent::from_exit("c", 1, 0, 1, "OOMKilled").is_none());
        // Clean exit.
        assert!(OomEvent::from_exit("c", 1, 0, 0, "Completed").is_none());
    }

    #[test]
    fn in_memory_source_push_then_pending() {
        let s = InMemoryOomSource::new();
        assert_eq!(s.pending(), 0);
        s.push(OomEvent::new("c-1", 1, 0));
        s.push(OomEvent::new("c-2", 2, 0));
        assert_eq!(s.pending(), 2);
    }

    #[tokio::test]
    async fn in_memory_source_next_drains_fifo() {
        let s = InMemoryOomSource::new();
        s.push(OomEvent::new("c-A", 1, -100));
        s.push(OomEvent::new("c-B", 2, -200));
        let a = s.next().await.unwrap();
        let b = s.next().await.unwrap();
        assert_eq!(a.container_id, "c-A");
        assert_eq!(b.container_id, "c-B");
        assert_eq!(s.pending(), 0);
    }

    #[tokio::test]
    async fn in_memory_source_next_blocks_until_push() {
        let s = Arc::new(InMemoryOomSource::new());
        let s2 = Arc::clone(&s);
        let join = tokio::spawn(async move { s2.next().await });
        // Give the spawned task a chance to park on `notified()`.
        tokio::task::yield_now().await;
        s.push(OomEvent::new("c-late", 7, 0));
        let got = join.await.unwrap().unwrap();
        assert_eq!(got.container_id, "c-late");
    }

    #[test]
    fn linux_cgroup_source_memory_events_path_layout() {
        let src = LinuxCgroupOomSource::new("/sys/fs/cgroup/kubepods.slice");
        let p = src.memory_events_path("c-deadbeef");
        assert_eq!(
            p,
            std::path::PathBuf::from(
                "/sys/fs/cgroup/kubepods.slice/c-deadbeef/memory.events"
            )
        );
        assert_eq!(
            src.cgroup_root(),
            std::path::Path::new("/sys/fs/cgroup/kubepods.slice")
        );
    }

    #[tokio::test]
    async fn linux_cgroup_source_next_returns_none_when_not_wired() {
        // The platform binding is not implemented in-module; ensure
        // the trait still drives the consumer to a clean shutdown.
        let src = LinuxCgroupOomSource::new("/sys/fs/cgroup");
        assert!(src.next().await.is_none());
    }

    #[test]
    fn bus_new_capacity_zero_clamps_to_one() {
        // broadcast::channel(0) would panic — the bus must clamp.
        let bus = OomEventBus::new(0);
        let _rx = bus.subscribe(); // does not panic
        assert_eq!(bus.subscriber_count(), 1);
    }

    #[test]
    fn bus_publish_without_subscribers_drops_event() {
        let bus = OomEventBus::new(16);
        let n = bus.publish(OomEvent::new("c", 1, 0));
        assert_eq!(n, 0);
    }

    #[tokio::test]
    async fn bus_fanout_to_multiple_subscribers() {
        let bus = OomEventBus::new(16);
        let mut rx1 = bus.subscribe();
        let mut rx2 = bus.subscribe();
        assert_eq!(bus.subscriber_count(), 2);

        let n = bus.publish(OomEvent::new("c-fanout", 99, -50));
        assert_eq!(n, 2);

        let e1 = rx1.recv().await.unwrap();
        let e2 = rx2.recv().await.unwrap();
        assert_eq!(e1.container_id, "c-fanout");
        assert_eq!(e2.container_id, "c-fanout");
        assert_eq!(e1.pid, 99);
        assert_eq!(e2.pid, 99);
    }

    #[tokio::test]
    async fn watcher_tick_forwards_source_to_bus() {
        let src = Arc::new(InMemoryOomSource::new());
        let bus = OomEventBus::new(16);
        let watcher = OomWatcher::new(src.clone(), bus.clone());
        let mut rx = bus.subscribe();

        src.push(OomEvent::new("c-tick", 1, -1000));
        let ok = watcher.tick().await;
        assert!(ok);

        let got = rx.recv().await.unwrap();
        assert_eq!(got.container_id, "c-tick");
        assert_eq!(got.oom_score_adj, -1000);
    }

    #[tokio::test]
    async fn watcher_run_exits_when_source_closes() {
        // LinuxCgroupOomSource::next always returns None → run() is a
        // no-op loop that terminates immediately.
        let src = Arc::new(LinuxCgroupOomSource::new("/sys/fs/cgroup"));
        let bus = OomEventBus::new(4);
        let watcher = OomWatcher::new(src, bus);
        // Should complete without hanging.
        tokio::time::timeout(std::time::Duration::from_secs(1), watcher.run())
            .await
            .expect("watcher must terminate when source is closed");
    }

    #[tokio::test]
    async fn watcher_bus_handle_is_shared() {
        let src = Arc::new(InMemoryOomSource::new());
        let bus = OomEventBus::new(8);
        let watcher = OomWatcher::new(src.clone(), bus.clone());

        // External subscriber.
        let mut external = bus.subscribe();
        // Subscriber via watcher.bus() handle.
        let mut via_watcher = watcher.bus().subscribe();

        src.push(OomEvent::new("c-share", 5, 0));
        assert!(watcher.tick().await);

        let a = external.recv().await.unwrap();
        let b = via_watcher.recv().await.unwrap();
        assert_eq!(a.container_id, "c-share");
        assert_eq!(b.container_id, "c-share");
    }
}

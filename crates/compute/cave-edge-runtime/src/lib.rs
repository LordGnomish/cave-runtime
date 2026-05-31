// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! cave-edge-runtime: a pure-Rust reimplementation of the edge-orchestration
//! decision logic from **K3s edge mode** (k3s-io/k3s) and **KubeEdge**
//! (kubeedge/kubeedge).
//!
//! The crate ports the *control logic* — pod-worker lifecycle, offline-first
//! metadata reconciliation, the MQTT-style event bus bridge, the reliable
//! cloud↔edge sync keeper, the device twin delta engine, the local-autonomy
//! connection state machine, and the constrained-resource (256 MB target)
//! admission/eviction model. Live transports (WebSocket/QUIC, the MQTT
//! broker, containerd CRI, real SQLite) stay out of scope and are documented
//! as such in `parity.manifest.toml`.
//!
//! Modules:
//!   edged        — minimal kubelet: pod-worker queue + phase + orphan GC + status cadence
//!   metamanager  — offline-first local metadata store (cache-through + serve-from-cache)
//!   eventbus     — MQTT-topic ↔ internal message bridge over a cave-streams local queue
//!   edgehub      — reliable cloud-edge sync keeper (msg IDs + ACK + retransmit + RV merge)
//!   devicetwin   — Expected/Actual twin state + version gating + delta computation

pub mod devicetwin;
pub mod edged;
pub mod edgehub;
pub mod error;
pub mod eventbus;
pub mod metamanager;

pub use devicetwin::{DeviceTwin, TwinDelta, TwinVersion};
pub use edged::{Edged, Pod, PodPhase};
pub use edgehub::{EdgeHub, RecvOutcome, SyncMessage};
pub use error::{EdgeError, Result};
pub use eventbus::{EdgeTopicKind, EventBus, Message, topic_matches};
pub use metamanager::{MetaManager, QueryOutcome};

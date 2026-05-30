// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Inter-agent communication — OpenJarvis multi-agent primitive.
//!
//! A local, in-process message bus so several named agents can coordinate
//! without a network: each registered agent owns an ordered inbox, messages
//! carry a monotonic sequence number for deterministic replay, and a thin
//! orchestrator fans a task out to a worker pool and gathers their replies.
//!
//! This is deliberately synchronous and single-process — the local-first
//! counterpart to the cluster-wide EventBus. Cross-process multi-agent
//! messaging defers to `cave-kernel`'s EventBus downstream.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::error::HermesError;

/// Coarse message intent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageKind {
    /// A unit of work delegated to a worker.
    Task,
    /// A reply carrying a worker's output.
    Result,
    /// Informational / coordination chatter.
    Info,
}

/// One message on the bus.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub seq: u64,
    pub from: String,
    pub to: String,
    pub kind: MessageKind,
    pub body: String,
}

/// In-process, synchronous message bus. Each registered agent owns an
/// ordered inbox; [`receive`](MessageBus::receive) drains it.
#[derive(Debug, Default)]
pub struct MessageBus {
    inboxes: BTreeMap<String, Vec<Message>>,
    next_seq: u64,
}

impl MessageBus {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register an agent (idempotent — re-spawning keeps the existing inbox).
    pub fn spawn(&mut self, name: &str) -> &mut Self {
        self.inboxes.entry(name.to_string()).or_default();
        self
    }

    pub fn is_registered(&self, name: &str) -> bool {
        self.inboxes.contains_key(name)
    }

    /// Deliver a message to `to`'s inbox. Errors if the recipient is not
    /// registered.
    pub fn send(
        &mut self,
        from: &str,
        to: &str,
        kind: MessageKind,
        body: impl Into<String>,
    ) -> crate::error::Result<u64> {
        if !self.inboxes.contains_key(to) {
            return Err(HermesError::Comms(format!("unknown recipient '{to}'")));
        }
        let seq = self.next_seq;
        self.next_seq += 1;
        let msg = Message {
            seq,
            from: from.to_string(),
            to: to.to_string(),
            kind,
            body: body.into(),
        };
        self.inboxes.get_mut(to).expect("checked above").push(msg);
        Ok(seq)
    }

    /// Send to every registered agent except the sender. Returns the
    /// recipient count.
    pub fn broadcast(
        &mut self,
        from: &str,
        kind: MessageKind,
        body: impl Into<String>,
    ) -> crate::error::Result<usize> {
        let body = body.into();
        let recipients: Vec<String> = self
            .inboxes
            .keys()
            .filter(|k| k.as_str() != from)
            .cloned()
            .collect();
        for to in &recipients {
            self.send(from, to, kind, body.clone())?;
        }
        Ok(recipients.len())
    }

    /// Number of undelivered messages in an agent's inbox.
    pub fn pending(&self, name: &str) -> usize {
        self.inboxes.get(name).map(Vec::len).unwrap_or(0)
    }

    /// Drain and return an agent's inbox in arrival order.
    pub fn receive(&mut self, name: &str) -> crate::error::Result<Vec<Message>> {
        let inbox = self
            .inboxes
            .get_mut(name)
            .ok_or_else(|| HermesError::Comms(format!("unknown agent '{name}'")))?;
        Ok(std::mem::take(inbox))
    }
}

/// Thin coordinator over a [`MessageBus`]: fans a task out to a worker pool
/// and gathers the replies addressed back to it.
#[derive(Debug)]
pub struct Orchestrator {
    coordinator: String,
    workers: Vec<String>,
    bus: MessageBus,
}

impl Orchestrator {
    pub fn new(coordinator: impl Into<String>) -> Self {
        let coordinator = coordinator.into();
        let mut bus = MessageBus::new();
        bus.spawn(&coordinator);
        Self {
            coordinator,
            workers: Vec::new(),
            bus,
        }
    }

    pub fn add_worker(&mut self, name: impl Into<String>) -> &mut Self {
        let name = name.into();
        self.bus.spawn(&name);
        self.workers.push(name);
        self
    }

    pub fn bus_mut(&mut self) -> &mut MessageBus {
        &mut self.bus
    }

    pub fn workers(&self) -> &[String] {
        &self.workers
    }

    /// Send `body` as a [`MessageKind::Task`] to every worker. Returns the
    /// number dispatched.
    pub fn delegate(&mut self, body: impl Into<String>) -> crate::error::Result<usize> {
        let body = body.into();
        let workers = self.workers.clone();
        for w in &workers {
            self.bus
                .send(&self.coordinator, w, MessageKind::Task, body.clone())?;
        }
        Ok(workers.len())
    }

    /// Drain the coordinator's inbox and return only the [`MessageKind::Result`]
    /// replies, in arrival order.
    pub fn collect_results(&mut self) -> crate::error::Result<Vec<Message>> {
        let coordinator = self.coordinator.clone();
        let all = self.bus.receive(&coordinator)?;
        Ok(all
            .into_iter()
            .filter(|m| m.kind == MessageKind::Result)
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_then_receive_in_order() {
        let mut bus = MessageBus::new();
        bus.spawn("a");
        bus.spawn("b");
        bus.send("a", "b", MessageKind::Info, "one").unwrap();
        bus.send("a", "b", MessageKind::Info, "two").unwrap();
        let inbox = bus.receive("b").unwrap();
        assert_eq!(inbox.len(), 2);
        assert_eq!(inbox[0].body, "one");
        assert_eq!(inbox[1].body, "two");
        assert!(inbox[0].seq < inbox[1].seq, "seq monotonic");
    }

    #[test]
    fn receive_drains_inbox() {
        let mut bus = MessageBus::new();
        bus.spawn("a");
        bus.spawn("b");
        bus.send("a", "b", MessageKind::Info, "x").unwrap();
        assert_eq!(bus.receive("b").unwrap().len(), 1);
        assert!(bus.receive("b").unwrap().is_empty(), "second receive is empty");
    }

    #[test]
    fn send_to_unknown_recipient_errors() {
        let mut bus = MessageBus::new();
        bus.spawn("a");
        let err = bus.send("a", "ghost", MessageKind::Info, "x").unwrap_err();
        assert!(matches!(err, crate::error::HermesError::Comms(_)));
    }

    #[test]
    fn pending_counts_undelivered() {
        let mut bus = MessageBus::new();
        bus.spawn("a");
        bus.spawn("b");
        assert_eq!(bus.pending("b"), 0);
        bus.send("a", "b", MessageKind::Task, "do it").unwrap();
        assert_eq!(bus.pending("b"), 1);
    }

    #[test]
    fn broadcast_reaches_everyone_but_sender() {
        let mut bus = MessageBus::new();
        bus.spawn("a");
        bus.spawn("b");
        bus.spawn("c");
        let n = bus.broadcast("a", MessageKind::Info, "hello").unwrap();
        assert_eq!(n, 2, "two recipients");
        assert_eq!(bus.pending("a"), 0, "sender does not receive own broadcast");
        assert_eq!(bus.pending("b"), 1);
        assert_eq!(bus.pending("c"), 1);
    }

    #[test]
    fn orchestrator_delegates_task_to_each_worker() {
        let mut orch = Orchestrator::new("coordinator");
        orch.add_worker("w1");
        orch.add_worker("w2");
        let dispatched = orch.delegate("build the report").unwrap();
        assert_eq!(dispatched, 2);
        assert_eq!(orch.bus_mut().pending("w1"), 1);
        let task = &orch.bus_mut().receive("w2").unwrap()[0];
        assert_eq!(task.kind, MessageKind::Task);
        assert_eq!(task.from, "coordinator");
        assert_eq!(task.body, "build the report");
    }

    #[test]
    fn orchestrator_collects_worker_results() {
        let mut orch = Orchestrator::new("coordinator");
        orch.add_worker("w1");
        orch.delegate("task").unwrap();
        // Worker replies with a Result addressed to the coordinator.
        orch.bus_mut()
            .send("w1", "coordinator", MessageKind::Result, "done")
            .unwrap();
        let results = orch.collect_results().unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].body, "done");
        assert_eq!(results[0].kind, MessageKind::Result);
    }
}

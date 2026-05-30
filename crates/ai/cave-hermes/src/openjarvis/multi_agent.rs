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

// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Rule engine — a port of the ThingsBoard rule-chain model: a directed
//! graph of *filter* / *transform* / *action* nodes wired by labelled
//! relations (`True`/`False` out of a filter, `Success` otherwise).
//!
//! A [`Message`] enters at the root node; each node either routes it onward
//! by a relation label or terminates it. Filter nodes evaluate a
//! [`Predicate`] over the message data; transform nodes mutate it in place;
//! action nodes record an [`ActionKind`] (save telemetry, push to a topic,
//! …). Self-loops are bounded by [`RuleChain::MAX_DEPTH`].

use crate::{KvMap, KvValue};
use std::collections::{BTreeMap, HashMap};

/// A message flowing through the rule chain.
#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub device_id: String,
    pub msg_type: String,
    pub data: KvMap,
    pub metadata: BTreeMap<String, String>,
}

impl Message {
    pub fn new(device_id: &str, msg_type: &str) -> Message {
        Message {
            device_id: device_id.to_string(),
            msg_type: msg_type.to_string(),
            data: KvMap::new(),
            metadata: BTreeMap::new(),
        }
    }
}

/// A boolean predicate over message data (filter-node condition).
#[derive(Debug, Clone, PartialEq)]
pub enum Predicate {
    Gt(String, f64),
    Lt(String, f64),
    Eq(String, KvValue),
    Exists(String),
    And(Box<Predicate>, Box<Predicate>),
    Or(Box<Predicate>, Box<Predicate>),
    Not(Box<Predicate>),
}

impl Predicate {
    pub fn eval(&self, msg: &Message) -> bool {
        match self {
            Predicate::Gt(k, n) => msg
                .data
                .get(k)
                .and_then(KvValue::as_f64)
                .is_some_and(|v| v > *n),
            Predicate::Lt(k, n) => msg
                .data
                .get(k)
                .and_then(KvValue::as_f64)
                .is_some_and(|v| v < *n),
            Predicate::Eq(k, val) => msg.data.get(k) == Some(val),
            Predicate::Exists(k) => msg.data.contains_key(k),
            Predicate::And(a, b) => a.eval(msg) && b.eval(msg),
            Predicate::Or(a, b) => a.eval(msg) || b.eval(msg),
            Predicate::Not(p) => !p.eval(msg),
        }
    }
}

/// An in-place message transformation (transform-node operation).
#[derive(Debug, Clone, PartialEq)]
pub enum TransformOp {
    SetConst(String, KvValue),
    Rename(String, String),
    Scale(String, f64),
    SetMetadata(String, String),
}

impl TransformOp {
    pub fn apply(&self, msg: &mut Message) {
        match self {
            TransformOp::SetConst(k, v) => {
                msg.data.insert(k.clone(), v.clone());
            }
            TransformOp::Rename(from, to) => {
                if let Some(v) = msg.data.remove(from) {
                    msg.data.insert(to.clone(), v);
                }
            }
            TransformOp::Scale(k, factor) => {
                if let Some(v) = msg.data.get(k).and_then(KvValue::as_f64) {
                    msg.data.insert(k.clone(), KvValue::Double(v * factor));
                }
            }
            TransformOp::SetMetadata(k, v) => {
                msg.metadata.insert(k.clone(), v.clone());
            }
        }
    }
}

/// The side effect an action node records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionKind {
    SaveTimeseries,
    PushToTopic(String),
    Log,
}

/// A rule-chain node.
#[derive(Debug, Clone, PartialEq)]
pub enum RuleNode {
    Filter { predicate: Predicate },
    Transform { op: TransformOp },
    Action { action: ActionKind },
}

/// The result of running a message through a chain.
#[derive(Debug, Clone, PartialEq)]
pub struct RuleOutcome {
    pub actions: Vec<ActionKind>,
    pub message: Message,
    pub visited: usize,
}

/// A directed rule chain.
#[derive(Debug, Default)]
pub struct RuleChain {
    nodes: Vec<RuleNode>,
    relations: HashMap<(usize, String), usize>,
    root: Option<usize>,
}

impl RuleChain {
    /// Hard cap on nodes visited per message — guards against relation cycles.
    pub const MAX_DEPTH: usize = 1000;

    pub fn new() -> RuleChain {
        RuleChain::default()
    }

    pub fn add_node(&mut self, node: RuleNode) -> usize {
        self.nodes.push(node);
        self.nodes.len() - 1
    }

    pub fn set_root(&mut self, idx: usize) {
        self.root = Some(idx);
    }

    pub fn link(&mut self, from: usize, label: &str, to: usize) {
        self.relations.insert((from, label.to_string()), to);
    }

    /// Run a message through the chain from the root.
    pub fn process(&self, mut msg: Message) -> RuleOutcome {
        let mut actions = Vec::new();
        let mut visited = 0usize;
        let mut current = self.root;
        while let Some(idx) = current {
            if visited >= Self::MAX_DEPTH {
                break;
            }
            visited += 1;
            let Some(node) = self.nodes.get(idx) else {
                break;
            };
            let label = match node {
                RuleNode::Filter { predicate } => {
                    if predicate.eval(&msg) {
                        "True"
                    } else {
                        "False"
                    }
                }
                RuleNode::Transform { op } => {
                    op.apply(&mut msg);
                    "Success"
                }
                RuleNode::Action { action } => {
                    actions.push(action.clone());
                    "Success"
                }
            };
            current = self.relations.get(&(idx, label.to_string())).copied();
        }
        RuleOutcome {
            actions,
            message: msg,
            visited,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::KvValue;

    fn msg(temp: f64) -> Message {
        let mut m = Message::new("dev-1", "POST_TELEMETRY");
        m.data.insert("temperature".into(), KvValue::Double(temp));
        m
    }

    #[test]
    fn predicate_gt_and_logic() {
        let p = Predicate::And(
            Box::new(Predicate::Gt("temperature".into(), 30.0)),
            Box::new(Predicate::Exists("temperature".into())),
        );
        assert!(p.eval(&msg(35.0)));
        assert!(!p.eval(&msg(10.0)));
        assert!(!Predicate::Eq("temperature".into(), KvValue::Double(1.0)).eval(&msg(2.0)));
    }

    #[test]
    fn transform_scale_and_rename() {
        let mut m = msg(20.0);
        TransformOp::Scale("temperature".into(), 1.8).apply(&mut m);
        // 20 * 1.8 = 36
        assert_eq!(m.data.get("temperature"), Some(&KvValue::Double(36.0)));
        TransformOp::Rename("temperature".into(), "temp_f".into()).apply(&mut m);
        assert!(m.data.get("temperature").is_none());
        assert_eq!(m.data.get("temp_f"), Some(&KvValue::Double(36.0)));
    }

    #[test]
    fn filter_routes_true_and_false_branches() {
        let mut chain = RuleChain::new();
        let root = chain.add_node(RuleNode::Filter {
            predicate: Predicate::Gt("temperature".into(), 30.0),
        });
        let hot = chain.add_node(RuleNode::Action {
            action: ActionKind::PushToTopic("alarms".into()),
        });
        let cold = chain.add_node(RuleNode::Action {
            action: ActionKind::SaveTimeseries,
        });
        chain.set_root(root);
        chain.link(root, "True", hot);
        chain.link(root, "False", cold);

        let hot_out = chain.process(msg(40.0));
        assert_eq!(
            hot_out.actions,
            vec![ActionKind::PushToTopic("alarms".into())]
        );
        let cold_out = chain.process(msg(10.0));
        assert_eq!(cold_out.actions, vec![ActionKind::SaveTimeseries]);
    }

    #[test]
    fn end_to_end_filter_transform_action() {
        let mut chain = RuleChain::new();
        let f = chain.add_node(RuleNode::Filter {
            predicate: Predicate::Exists("temperature".into()),
        });
        let t = chain.add_node(RuleNode::Transform {
            op: TransformOp::SetMetadata("processed".into(), "yes".into()),
        });
        let a = chain.add_node(RuleNode::Action {
            action: ActionKind::SaveTimeseries,
        });
        chain.set_root(f);
        chain.link(f, "True", t);
        chain.link(t, "Success", a);

        let out = chain.process(msg(22.0));
        assert_eq!(out.actions, vec![ActionKind::SaveTimeseries]);
        assert_eq!(
            out.message.metadata.get("processed").map(String::as_str),
            Some("yes")
        );
    }

    #[test]
    fn unmatched_relation_drops_message_without_action() {
        let mut chain = RuleChain::new();
        let root = chain.add_node(RuleNode::Filter {
            predicate: Predicate::Gt("temperature".into(), 30.0),
        });
        chain.set_root(root);
        // Only a True branch is linked; a cold message has nowhere to go.
        let sink = chain.add_node(RuleNode::Action {
            action: ActionKind::Log,
        });
        chain.link(root, "True", sink);
        let out = chain.process(msg(5.0));
        assert!(out.actions.is_empty());
    }

    #[test]
    fn cycle_is_bounded_by_max_depth() {
        let mut chain = RuleChain::new();
        let t = chain.add_node(RuleNode::Transform {
            op: TransformOp::SetMetadata("x".into(), "1".into()),
        });
        chain.set_root(t);
        // Self-loop on Success — must terminate via the depth guard, not hang.
        chain.link(t, "Success", t);
        let out = chain.process(msg(1.0));
        assert!(out.visited <= RuleChain::MAX_DEPTH);
    }
}

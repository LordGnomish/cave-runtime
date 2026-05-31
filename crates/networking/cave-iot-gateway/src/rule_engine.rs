// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Rule engine. (RED: chain processing pending.)

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
        let hot = chain.add_node(RuleNode::Action { action: ActionKind::PushToTopic("alarms".into()) });
        let cold = chain.add_node(RuleNode::Action { action: ActionKind::SaveTimeseries });
        chain.set_root(root);
        chain.link(root, "True", hot);
        chain.link(root, "False", cold);

        let hot_out = chain.process(msg(40.0));
        assert_eq!(hot_out.actions, vec![ActionKind::PushToTopic("alarms".into())]);
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
        let a = chain.add_node(RuleNode::Action { action: ActionKind::SaveTimeseries });
        chain.set_root(f);
        chain.link(f, "True", t);
        chain.link(t, "Success", a);

        let out = chain.process(msg(22.0));
        assert_eq!(out.actions, vec![ActionKind::SaveTimeseries]);
        assert_eq!(out.message.metadata.get("processed").map(String::as_str), Some("yes"));
    }

    #[test]
    fn unmatched_relation_drops_message_without_action() {
        let mut chain = RuleChain::new();
        let root = chain.add_node(RuleNode::Filter {
            predicate: Predicate::Gt("temperature".into(), 30.0),
        });
        chain.set_root(root);
        // Only a True branch is linked; a cold message has nowhere to go.
        let sink = chain.add_node(RuleNode::Action { action: ActionKind::Log });
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

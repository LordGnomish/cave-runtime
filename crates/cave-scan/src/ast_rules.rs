// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! AST-based rule engine — parity with
//! `sonar-scanner-engine/src/main/java/org/sonar/scanner/sca/AstRuleEngine.java`
//! (SonarQube v10.4.1).
//!
//! Rules operate over a tiny shared AST (function decls, calls, string
//! literals, conditionals). Real per-language frontends (tree-sitter
//! grammars, JavaParser, etc.) live behind cave-cli; this module is the
//! rule engine that consumes whatever AST those frontends emit.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AstNode {
    FunctionDecl {
        name: String,
        params: Vec<String>,
        body: Vec<AstNode>,
    },
    Call {
        callee: String,
        args: Vec<AstNode>,
    },
    StringLit(String),
    NumberLit(i64),
    Ident(String),
    If {
        cond: Box<AstNode>,
        then_branch: Vec<AstNode>,
        else_branch: Vec<AstNode>,
    },
    Return(Box<AstNode>),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AstRule {
    pub id: String,
    pub message: String,
    pub check: AstCheck,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AstCheck {
    /// Match any `Call` node whose callee matches a given name.
    CalleeNamed(String),
    /// Match any `StringLit` containing the substring.
    StringContains(String),
    /// Match any `FunctionDecl` with parameter count >= threshold.
    FunctionParamsAtLeast(usize),
    /// Logical AND of nested checks.
    All(Vec<AstCheck>),
    /// Logical OR of nested checks.
    Any(Vec<AstCheck>),
    /// Logical NOT of nested check.
    Not(Box<AstCheck>),
    /// `if cond` that contains no `else`.
    IfWithoutElse,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AstMatch {
    pub rule_id: String,
    pub path: Vec<String>,
}

pub fn run_rule(rule: &AstRule, tree: &[AstNode]) -> Vec<AstMatch> {
    let mut out = Vec::new();
    for node in tree {
        walk(node, &mut Vec::new(), &rule.check, rule, &mut out);
    }
    out
}

fn walk(
    node: &AstNode,
    path: &mut Vec<String>,
    check: &AstCheck,
    rule: &AstRule,
    out: &mut Vec<AstMatch>,
) {
    if eval_check(node, check) {
        out.push(AstMatch {
            rule_id: rule.id.clone(),
            path: path.clone(),
        });
    }
    match node {
        AstNode::FunctionDecl { name, body, .. } => {
            path.push(format!("fn:{}", name));
            for child in body {
                walk(child, path, check, rule, out);
            }
            path.pop();
        }
        AstNode::Call { callee, args } => {
            path.push(format!("call:{}", callee));
            for child in args {
                walk(child, path, check, rule, out);
            }
            path.pop();
        }
        AstNode::If {
            cond,
            then_branch,
            else_branch,
        } => {
            path.push("if".into());
            walk(cond, path, check, rule, out);
            for child in then_branch {
                walk(child, path, check, rule, out);
            }
            for child in else_branch {
                walk(child, path, check, rule, out);
            }
            path.pop();
        }
        AstNode::Return(inner) => walk(inner, path, check, rule, out),
        AstNode::StringLit(_) | AstNode::NumberLit(_) | AstNode::Ident(_) => {}
    }
}

fn eval_check(node: &AstNode, check: &AstCheck) -> bool {
    match check {
        AstCheck::CalleeNamed(name) => matches!(node, AstNode::Call { callee, .. } if callee == name),
        AstCheck::StringContains(needle) => {
            matches!(node, AstNode::StringLit(s) if s.contains(needle.as_str()))
        }
        AstCheck::FunctionParamsAtLeast(n) => {
            matches!(node, AstNode::FunctionDecl { params, .. } if params.len() >= *n)
        }
        AstCheck::All(cs) => cs.iter().all(|c| eval_check(node, c)),
        AstCheck::Any(cs) => cs.iter().any(|c| eval_check(node, c)),
        AstCheck::Not(inner) => !eval_check(node, inner),
        AstCheck::IfWithoutElse => {
            matches!(node, AstNode::If { else_branch, .. } if else_branch.is_empty())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(name: &str, args: Vec<AstNode>) -> AstNode {
        AstNode::Call {
            callee: name.into(),
            args,
        }
    }

    fn func(name: &str, params: Vec<&str>, body: Vec<AstNode>) -> AstNode {
        AstNode::FunctionDecl {
            name: name.into(),
            params: params.into_iter().map(String::from).collect(),
            body,
        }
    }

    fn rule(id: &str, check: AstCheck) -> AstRule {
        AstRule {
            id: id.into(),
            message: "m".into(),
            check,
        }
    }

    #[test]
    fn callee_named_top_level() {
        let tree = vec![call("eval", vec![])];
        let m = run_rule(&rule("no-eval", AstCheck::CalleeNamed("eval".into())), &tree);
        assert_eq!(m.len(), 1);
    }

    #[test]
    fn callee_named_nested() {
        let tree = vec![func(
            "outer",
            vec![],
            vec![call("eval", vec![AstNode::StringLit("hello".into())])],
        )];
        let m = run_rule(&rule("no-eval", AstCheck::CalleeNamed("eval".into())), &tree);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].path, vec!["fn:outer"]);
    }

    #[test]
    fn string_contains_finds_credentials() {
        let tree = vec![call(
            "log",
            vec![AstNode::StringLit("password=hunter2".into())],
        )];
        let m = run_rule(
            &rule(
                "secret",
                AstCheck::StringContains("password=".into()),
            ),
            &tree,
        );
        assert_eq!(m.len(), 1);
    }

    #[test]
    fn function_params_at_least() {
        let tree = vec![
            func("small", vec!["a", "b"], vec![]),
            func("big", vec!["a", "b", "c", "d", "e"], vec![]),
        ];
        let m = run_rule(
            &rule("too-many-params", AstCheck::FunctionParamsAtLeast(5)),
            &tree,
        );
        assert_eq!(m.len(), 1);
    }

    #[test]
    fn all_combinator() {
        let tree = vec![call("log", vec![AstNode::StringLit("token=abc".into())])];
        let m = run_rule(
            &rule(
                "log-token",
                AstCheck::All(vec![
                    AstCheck::CalleeNamed("log".into()),
                    AstCheck::Any(vec![
                        AstCheck::CalleeNamed("log".into()),
                        AstCheck::StringContains("token".into()),
                    ]),
                ]),
            ),
            &tree,
        );
        assert_eq!(m.len(), 1);
    }

    #[test]
    fn not_combinator() {
        let tree = vec![call("safe_call", vec![])];
        let m = run_rule(
            &rule(
                "not-eval",
                AstCheck::Not(Box::new(AstCheck::CalleeNamed("eval".into()))),
            ),
            &tree,
        );
        assert!(!m.is_empty());
    }

    #[test]
    fn if_without_else_detected() {
        let tree = vec![AstNode::If {
            cond: Box::new(AstNode::Ident("x".into())),
            then_branch: vec![call("foo", vec![])],
            else_branch: vec![],
        }];
        let m = run_rule(&rule("if-no-else", AstCheck::IfWithoutElse), &tree);
        assert_eq!(m.len(), 1);
    }

    #[test]
    fn if_with_else_not_detected() {
        let tree = vec![AstNode::If {
            cond: Box::new(AstNode::Ident("x".into())),
            then_branch: vec![call("foo", vec![])],
            else_branch: vec![call("bar", vec![])],
        }];
        let m = run_rule(&rule("if-no-else", AstCheck::IfWithoutElse), &tree);
        assert!(m.is_empty());
    }

    #[test]
    fn empty_tree_returns_empty() {
        let m = run_rule(
            &rule("any", AstCheck::CalleeNamed("eval".into())),
            &[],
        );
        assert!(m.is_empty());
    }
}

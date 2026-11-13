// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Common subexpression elimination (CSE) analysis.
//!
//! Upstream: `crates/datafusion-common/src/cse.rs`
//!
//! DataFusion's CSE pass walks a forest of expression trees, assigns each
//! node a structural identifier, counts how often each identifier occurs,
//! and reports the *compound* subexpressions that occur more than once.
//! The optimizer then hoists each such subexpression into a single shared
//! evaluation (a `WITH`-style common column) so it is computed once
//! instead of once per occurrence.
//!
//! Upstream uses a bottom-up `TreeNodeVisitor` (`ExprIdentifierVisitor`)
//! that builds an `IdArray` of structural identifiers and a
//! `expr_stats: HashMap<Identifier, (count, …)>` occurrence map, then a
//! second pass (`is_valid` / `CommonNodes`) selects the nodes whose count
//! exceeds one and that are not trivial leaves
//! (`Expr::can_avoid_cse` excludes columns/literals). This port carries
//! that exact analysis over the cave-datafusion `LogicalExpr` AST.
//!
//! Notes on faithfulness to upstream:
//!   * Identifier — upstream hashes the node plus a per-node "is normal /
//!     conditional" flag. Our `LogicalExpr` has no conditional (CASE/AND
//!     short-circuit) nodes yet, so every node is "normal"; we use the
//!     structural value of the node itself as its identifier (two nodes
//!     are the same iff they are structurally equal), which is exactly
//!     upstream's behavior for the normal-evaluation subset.
//!   * Leaf exclusion — upstream's `Expr::can_avoid_cse()` returns `true`
//!     for `Column`, `Literal`, and similarly cost-free nodes; CSE never
//!     hoists them. We exclude `Column` and `Literal` for the same reason.
//!   * Ordering — upstream reports common nodes deterministically; we
//!     report them in first-seen (pre-order) order, deduplicated.

use crate::logical_expr::LogicalExpr;

/// Result of running the CSE analysis over a forest of expressions.
///
/// Mirrors the state upstream's `CSE`/`ExprIdentifierVisitor` accumulates:
/// an occurrence count per distinct subexpression plus the list of
/// hoist-worthy common subexpressions.
#[derive(Debug, Clone, Default)]
pub struct CommonSubexprAnalysis {
    /// (subexpression, occurrence-count), in first-seen pre-order.
    stats: Vec<(LogicalExpr, usize)>,
}

impl CommonSubexprAnalysis {
    /// Analyze a forest of expressions: count every distinct subexpression
    /// occurrence across all the trees.
    ///
    /// Port of the first (identifier/visit) phase of upstream
    /// `CSE::find_common_exprs` — `ExprIdentifierVisitor::f_up` increments
    /// `expr_stats[id].0` for each visited node.
    pub fn analyze(exprs: &[LogicalExpr]) -> Self {
        let mut analysis = Self::default();
        for e in exprs {
            analysis.visit(e);
        }
        analysis
    }

    /// Bottom-up visit that bumps the occurrence count of every node in
    /// the tree (the node itself and, recursively, its children).
    fn visit(&mut self, expr: &LogicalExpr) {
        // Recurse into children first (bottom-up, as upstream does).
        match expr {
            LogicalExpr::Column { .. } | LogicalExpr::Literal { .. } => {}
            LogicalExpr::BinaryOp { left, right, .. } => {
                self.visit(left);
                self.visit(right);
            }
            LogicalExpr::Not { expr }
            | LogicalExpr::IsNull { expr }
            | LogicalExpr::IsNotNull { expr }
            | LogicalExpr::Cast { expr, .. }
            | LogicalExpr::Alias { expr, .. } => self.visit(expr),
            LogicalExpr::Function { args, .. } => {
                for a in args {
                    self.visit(a);
                }
            }
        }
        self.bump(expr);
    }

    /// Increment the occurrence count for a single subexpression's
    /// structural identifier.
    fn bump(&mut self, expr: &LogicalExpr) {
        if let Some(slot) = self.stats.iter_mut().find(|(e, _)| e == expr) {
            slot.1 += 1;
        } else {
            self.stats.push((expr.clone(), 1));
        }
    }

    /// How many times the exact subexpression `expr` occurred across the
    /// analyzed forest. Zero if it never appeared.
    pub fn occurrences(&self, expr: &LogicalExpr) -> usize {
        self.stats
            .iter()
            .find(|(e, _)| e == expr)
            .map(|(_, c)| *c)
            .unwrap_or(0)
    }

    /// The hoist-worthy common subexpressions: those that occurred more
    /// than once and are not trivial leaves (columns / literals), in
    /// first-seen order.
    ///
    /// Port of upstream's `CSE::find_common_exprs` selection — a node is
    /// chosen when `count > 1` and `!node.can_avoid_cse()`.
    pub fn common_exprs(&self) -> Vec<LogicalExpr> {
        self.stats
            .iter()
            .filter(|(e, c)| *c > 1 && !Self::can_avoid_cse(e))
            .map(|(e, _)| e.clone())
            .collect()
    }

    /// Whether a node is cost-free enough that CSE should never hoist it.
    ///
    /// Port of upstream `Expr::can_avoid_cse()` — column references and
    /// literals are free to re-evaluate, so they are excluded from the
    /// common-subexpression set regardless of how often they appear.
    fn can_avoid_cse(expr: &LogicalExpr) -> bool {
        matches!(
            expr,
            LogicalExpr::Column { .. } | LogicalExpr::Literal { .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logical_expr::BinaryOp;

    #[test]
    fn leaves_excluded_even_when_repeated() {
        // a + a — the column `a` appears twice but is a leaf; the whole
        // expression appears once.
        let a = LogicalExpr::col("a");
        let whole = LogicalExpr::binary(a.clone(), BinaryOp::Plus, a.clone());
        let analysis = CommonSubexprAnalysis::analyze(std::slice::from_ref(&whole));
        assert_eq!(analysis.occurrences(&a), 2);
        assert!(analysis.common_exprs().is_empty());
    }

    #[test]
    fn empty_forest_is_empty() {
        let analysis = CommonSubexprAnalysis::analyze(&[]);
        assert!(analysis.common_exprs().is_empty());
        assert_eq!(analysis.occurrences(&LogicalExpr::col("x")), 0);
    }
}

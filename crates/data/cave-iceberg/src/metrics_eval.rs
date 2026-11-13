// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Inclusive metrics evaluator — manifest-time data-file pruning.
//!
//! Upstream: `crates/iceberg/src/expr/visitors/inclusive_metrics_evaluator.rs`
//! Spec: <https://iceberg.apache.org/spec/#scan-planning>
//!
//! This is the core data-skipping algorithm. Given a predicate and a
//! [`DataFile`]'s per-column statistics — value counts, null counts and
//! lower/upper bounds — it decides whether the file *might* contain rows
//! that satisfy the predicate. It never reads the file. It returns:
//!
//! * `true`  — `ROWS_MIGHT_MATCH`: the file must be scanned.
//! * `false` — `ROWS_CANNOT_MATCH`: the file can be safely pruned.
//!
//! The evaluator is "inclusive": when in doubt (missing stats, undecidable
//! comparison) it returns `true` so a file is never wrongly dropped. This
//! mirrors upstream's `ROWS_MIGHT_MATCH`/`ROWS_CANNOT_MATCH` constants and
//! the conservative defaults in `InclusiveMetricsEvaluator::visit_*`.
//!
//! cave-iceberg's [`crate::expr::Reference`] is name-based (field-ids are
//! resolved at bind time elsewhere), so the evaluator is constructed with
//! the single bound field-id that the predicate's column maps to. This
//! keeps the algorithm a faithful line-port while matching the crate's
//! existing predicate model. Compound predicates (And/Or/Not) walk the
//! AST exactly as upstream's visitor does.
//!
//! 2026-05-30 — Wave-4 honest TDD conversion (parity partial #3 → mapped).

use crate::expr::{CompareOp, Predicate, Term};
use crate::manifest::DataFile;
use serde_json::Value as Json;
use std::cmp::Ordering;

/// upstream constant: the file might contain matching rows → scan it.
const ROWS_MIGHT_MATCH: bool = true;
/// upstream constant: the file cannot contain matching rows → prune it.
const ROWS_CANNOT_MATCH: bool = false;

/// Decoded bound — the in-memory comparable form of an Iceberg single-value
/// bound. Only the primitive shapes the scan planner needs are modelled
/// (long/double/string); other types decode to `None` → conservatively kept.
#[derive(Debug, Clone, PartialEq)]
enum Bound {
    Int(i64),
    Float(f64),
    Str(String),
}

impl Bound {
    fn partial_cmp(&self, other: &Bound) -> Option<Ordering> {
        match (self, other) {
            (Bound::Int(a), Bound::Int(b)) => Some(a.cmp(b)),
            (Bound::Float(a), Bound::Float(b)) => a.partial_cmp(b),
            (Bound::Int(a), Bound::Float(b)) => (*a as f64).partial_cmp(b),
            (Bound::Float(a), Bound::Int(b)) => a.partial_cmp(&(*b as f64)),
            (Bound::Str(a), Bound::Str(b)) => Some(a.cmp(b)),
            _ => None,
        }
    }
}

/// Decode a JSON literal (the predicate's right-hand side) into a [`Bound`].
fn bound_from_json(v: &Json) -> Option<Bound> {
    match v {
        Json::Number(n) => {
            if let Some(i) = n.as_i64() {
                Some(Bound::Int(i))
            } else {
                n.as_f64().map(Bound::Float)
            }
        }
        Json::String(s) => Some(Bound::Str(s.clone())),
        _ => None,
    }
}

/// Decode a manifest lower/upper bound. The wire form is the Iceberg
/// single-value binary serialization, hex-encoded. We decode the primitive
/// encodings the scan planner needs:
///
/// * 8-byte little-endian → long (`Bound::Int`)
/// * 4-byte little-endian → int   (`Bound::Int`)
/// * 8-byte little-endian IEEE-754 → double, when it is not also a valid
///   long context (we let the predicate literal's type disambiguate).
/// * otherwise UTF-8 → string (`Bound::Str`).
///
/// `literal` is the predicate literal used as a type hint so an 8-byte
/// bound is read as a long when compared to an integer and as a double
/// when compared to a float — matching how Iceberg stores per-type bounds.
fn decode_bound(hex: &str, literal: &Bound) -> Option<Bound> {
    let bytes = decode_hex(hex)?;
    match literal {
        Bound::Int(_) => match bytes.len() {
            8 => {
                let mut a = [0u8; 8];
                a.copy_from_slice(&bytes);
                Some(Bound::Int(i64::from_le_bytes(a)))
            }
            4 => {
                let mut a = [0u8; 4];
                a.copy_from_slice(&bytes);
                Some(Bound::Int(i32::from_le_bytes(a) as i64))
            }
            _ => None,
        },
        Bound::Float(_) => match bytes.len() {
            8 => {
                let mut a = [0u8; 8];
                a.copy_from_slice(&bytes);
                Some(Bound::Float(f64::from_le_bytes(a)))
            }
            4 => {
                let mut a = [0u8; 4];
                a.copy_from_slice(&bytes);
                Some(Bound::Float(f32::from_le_bytes(a) as f64))
            }
            _ => None,
        },
        Bound::Str(_) => String::from_utf8(bytes).ok().map(Bound::Str),
    }
}

fn decode_hex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let hi = (bytes[i] as char).to_digit(16)?;
        let lo = (bytes[i + 1] as char).to_digit(16)?;
        out.push(((hi << 4) | lo) as u8);
        i += 2;
    }
    Some(out)
}

/// Evaluates a predicate against a single [`DataFile`]'s column statistics.
///
/// Faithful port of upstream
/// `InclusiveMetricsEvaluator::eval` + its `BoundPredicateVisitor` impl.
pub struct InclusiveMetricsEvaluator<'a> {
    predicate: &'a Predicate,
    /// Field-id the predicate's referenced column binds to in this file.
    field_id: i32,
}

impl<'a> InclusiveMetricsEvaluator<'a> {
    /// Construct an evaluator for `predicate` whose referenced column is
    /// bound to `field_id` in the target manifests.
    pub fn new(predicate: &'a Predicate, field_id: i32) -> Self {
        Self {
            predicate,
            field_id,
        }
    }

    /// `true` → the file might contain matching rows (scan it);
    /// `false` → the file cannot contain matching rows (prune it).
    pub fn eval(&self, file: &DataFile) -> bool {
        // upstream short-circuits on empty files.
        if file.record_count == 0 {
            return ROWS_CANNOT_MATCH;
        }
        self.visit(self.predicate, file)
    }

    fn visit(&self, p: &Predicate, file: &DataFile) -> bool {
        match p {
            Predicate::True => ROWS_MIGHT_MATCH,
            Predicate::False => ROWS_CANNOT_MATCH,
            Predicate::And(a, b) => {
                // upstream: AND → result_left && result_right
                self.visit(a, file) && self.visit(b, file)
            }
            Predicate::Or(a, b) => {
                // upstream: OR → result_left || result_right
                self.visit(a, file) || self.visit(b, file)
            }
            Predicate::Not(inner) => {
                // Iceberg rewrites NOT during binding; here we evaluate
                // the child inclusively and negate the decidable case.
                // Inclusive negation: a child that "cannot match" means the
                // NOT might match; otherwise be conservative.
                !self.visit_strict(inner, file)
            }
            Predicate::Compare { op, left, right } => self.visit_compare(*op, left, right, file),
        }
    }

    /// Strict variant used under NOT: returns `true` only when the child is
    /// *certain* to match every row, else `false`. With only inclusive
    /// metrics that certainty is unavailable, so we conservatively return
    /// `false` (→ NOT keeps the file). Mirrors upstream's conservative
    /// handling of NotNull/negation on bounds-only stats.
    fn visit_strict(&self, _p: &Predicate, _file: &DataFile) -> bool {
        ROWS_CANNOT_MATCH
    }

    fn visit_compare(&self, op: CompareOp, left: &Term, right: &Term, file: &DataFile) -> bool {
        let fid = self.field_id;
        let value_count = file.value_counts.get(&fid).copied();
        let null_count = file.null_value_counts.get(&fid).copied();

        match op {
            CompareOp::IsNull => {
                // upstream visit_is_null: if no nulls recorded → cannot match.
                match null_count {
                    Some(0) => ROWS_CANNOT_MATCH,
                    _ => ROWS_MIGHT_MATCH,
                }
            }
            CompareOp::IsNotNull => {
                // upstream visit_not_null: if every value is null → cannot match.
                match (value_count, null_count) {
                    (Some(vc), Some(nc)) if vc == nc => ROWS_CANNOT_MATCH,
                    _ => ROWS_MIGHT_MATCH,
                }
            }
            _ => {
                // Decode the predicate literal once.
                let lit = match literal_of(left, right).and_then(bound_from_json) {
                    Some(l) => l,
                    None => return ROWS_MIGHT_MATCH, // undecidable → keep
                };

                // If the column is entirely null, value comparisons cannot
                // match (matches upstream `contains_nulls_only` early-out).
                if let (Some(vc), Some(nc)) = (value_count, null_count) {
                    if vc == nc {
                        return ROWS_CANNOT_MATCH;
                    }
                }

                let lower = file
                    .lower_bounds
                    .get(&fid)
                    .and_then(|h| decode_bound(h, &lit));
                let upper = file
                    .upper_bounds
                    .get(&fid)
                    .and_then(|h| decode_bound(h, &lit));

                self.visit_bounded(op, &lit, lower.as_ref(), upper.as_ref())
            }
        }
    }

    /// The per-operator bound test — direct port of upstream
    /// `visit_less_than` / `visit_greater_than` / `visit_equal` etc.
    fn visit_bounded(
        &self,
        op: CompareOp,
        lit: &Bound,
        lower: Option<&Bound>,
        upper: Option<&Bound>,
    ) -> bool {
        match op {
            CompareOp::Less | CompareOp::LessOrEqual => {
                // upstream visit_less_than: if lower_bound >= literal (strict)
                // the file cannot contain a smaller value.
                if let Some(lb) = lower {
                    if let Some(ord) = lb.partial_cmp(lit) {
                        let cannot = match op {
                            CompareOp::Less => ord != Ordering::Less, // lower >= lit
                            CompareOp::LessOrEqual => ord == Ordering::Greater, // lower > lit
                            _ => unreachable!(),
                        };
                        if cannot {
                            return ROWS_CANNOT_MATCH;
                        }
                    }
                }
                ROWS_MIGHT_MATCH
            }
            CompareOp::Greater | CompareOp::GreaterOrEqual => {
                // upstream visit_greater_than: if upper_bound <= literal the
                // file cannot contain a larger value.
                if let Some(ub) = upper {
                    if let Some(ord) = ub.partial_cmp(lit) {
                        let cannot = match op {
                            CompareOp::Greater => ord != Ordering::Greater, // upper <= lit
                            CompareOp::GreaterOrEqual => ord == Ordering::Less, // upper < lit
                            _ => unreachable!(),
                        };
                        if cannot {
                            return ROWS_CANNOT_MATCH;
                        }
                    }
                }
                ROWS_MIGHT_MATCH
            }
            CompareOp::Equal => {
                // upstream visit_equal: literal must fall within [lower, upper].
                if let Some(lb) = lower {
                    if let Some(Ordering::Less) = lit.partial_cmp(lb) {
                        return ROWS_CANNOT_MATCH; // lit < lower
                    }
                }
                if let Some(ub) = upper {
                    if let Some(Ordering::Greater) = lit.partial_cmp(ub) {
                        return ROWS_CANNOT_MATCH; // lit > upper
                    }
                }
                ROWS_MIGHT_MATCH
            }
            CompareOp::NotEqual => {
                // upstream visit_not_equal: bounds cannot prove absence of a
                // value, so a NotEqual is always kept.
                ROWS_MIGHT_MATCH
            }
            CompareOp::IsNull | CompareOp::IsNotNull => unreachable!(),
        }
    }
}

/// Return whichever side of a comparison is the literal value.
fn literal_of<'a>(left: &'a Term, right: &'a Term) -> Option<&'a Json> {
    match (left, right) {
        (_, Term::Literal(v)) => Some(v),
        (Term::Literal(v), _) => Some(v),
        _ => None,
    }
}

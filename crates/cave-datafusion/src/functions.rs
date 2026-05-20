// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! SQL function registry — scalar + aggregate.
//!
//! Upstream:
//! * `crates/datafusion-functions/src/lib.rs` (scalar built-ins)
//! * `crates/datafusion-functions-aggregate/src/lib.rs`
//!
//! The MVP registers the canonical small set: `abs / coalesce / length /
//! lower / upper / concat / round / least / greatest` (scalar), and
//! `count / sum / avg / min / max` (aggregate). Window functions are
//! deferred — see `[[scope_cuts]] window-functions`.

use crate::error::{Error, Result};
use crate::row::Value;
use std::collections::HashMap;
use std::sync::Arc;

pub type ScalarFn = Arc<dyn Fn(&[Value]) -> Result<Value> + Send + Sync>;

#[derive(Clone)]
pub struct ScalarFunction {
    pub name: String,
    pub fun: ScalarFn,
}

impl std::fmt::Debug for ScalarFunction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScalarFunction")
            .field("name", &self.name)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AggregateKind {
    Count,
    Sum,
    Avg,
    Min,
    Max,
}

impl AggregateKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Count => "count",
            Self::Sum => "sum",
            Self::Avg => "avg",
            Self::Min => "min",
            Self::Max => "max",
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct FunctionRegistry {
    scalars: HashMap<String, ScalarFunction>,
    aggregates: HashMap<String, AggregateKind>,
}

impl FunctionRegistry {
    pub fn new() -> Self {
        let mut r = Self::default();
        r.install_default_scalars();
        r.install_default_aggregates();
        r
    }

    pub fn lookup_scalar(&self, name: &str) -> Option<&ScalarFunction> {
        self.scalars.get(&name.to_lowercase())
    }

    pub fn lookup_aggregate(&self, name: &str) -> Option<AggregateKind> {
        self.aggregates.get(&name.to_lowercase()).copied()
    }

    pub fn register_scalar(&mut self, f: ScalarFunction) {
        self.scalars.insert(f.name.to_lowercase(), f);
    }

    pub fn scalar_names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.scalars.keys().cloned().collect();
        v.sort();
        v
    }

    pub fn aggregate_names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.aggregates.keys().cloned().collect();
        v.sort();
        v
    }

    fn install_default_scalars(&mut self) {
        let mut register = |name: &str, fun: ScalarFn| {
            self.scalars.insert(
                name.to_string(),
                ScalarFunction {
                    name: name.to_string(),
                    fun,
                },
            );
        };

        register(
            "abs",
            Arc::new(|args| {
                require_arity("abs", args, 1)?;
                Ok(match &args[0] {
                    Value::Int32(v) => Value::Int32(v.abs()),
                    Value::Int64(v) => Value::Int64(v.abs()),
                    Value::Float64(v) => Value::Float64(v.abs()),
                    Value::Null => Value::Null,
                    _ => return Err(Error::TypeMismatch("abs requires numeric".into())),
                })
            }),
        );
        register(
            "coalesce",
            Arc::new(|args| {
                for v in args {
                    if !v.is_null() {
                        return Ok(v.clone());
                    }
                }
                Ok(Value::Null)
            }),
        );
        register(
            "length",
            Arc::new(|args| {
                require_arity("length", args, 1)?;
                Ok(match &args[0] {
                    Value::Utf8(s) => Value::Int64(s.chars().count() as i64),
                    Value::Null => Value::Null,
                    _ => return Err(Error::TypeMismatch("length requires string".into())),
                })
            }),
        );
        register(
            "lower",
            Arc::new(|args| {
                require_arity("lower", args, 1)?;
                Ok(match &args[0] {
                    Value::Utf8(s) => Value::Utf8(s.to_lowercase()),
                    Value::Null => Value::Null,
                    _ => return Err(Error::TypeMismatch("lower requires string".into())),
                })
            }),
        );
        register(
            "upper",
            Arc::new(|args| {
                require_arity("upper", args, 1)?;
                Ok(match &args[0] {
                    Value::Utf8(s) => Value::Utf8(s.to_uppercase()),
                    Value::Null => Value::Null,
                    _ => return Err(Error::TypeMismatch("upper requires string".into())),
                })
            }),
        );
        register(
            "concat",
            Arc::new(|args| {
                let mut out = String::new();
                for a in args {
                    match a {
                        Value::Null => {}
                        Value::Utf8(s) => out.push_str(s),
                        Value::Int32(n) => out.push_str(&n.to_string()),
                        Value::Int64(n) => out.push_str(&n.to_string()),
                        Value::Float64(n) => out.push_str(&n.to_string()),
                        Value::Bool(b) => out.push_str(&b.to_string()),
                    }
                }
                Ok(Value::Utf8(out))
            }),
        );
        register(
            "round",
            Arc::new(|args| {
                require_arity("round", args, 1)?;
                Ok(match &args[0] {
                    Value::Float64(v) => Value::Float64(v.round()),
                    Value::Int32(v) => Value::Int32(*v),
                    Value::Int64(v) => Value::Int64(*v),
                    Value::Null => Value::Null,
                    _ => return Err(Error::TypeMismatch("round requires numeric".into())),
                })
            }),
        );
        register(
            "least",
            Arc::new(|args| {
                if args.is_empty() {
                    return Err(Error::TypeMismatch("least requires >=1 arg".into()));
                }
                let mut best: Option<&Value> = None;
                for v in args {
                    if v.is_null() {
                        continue;
                    }
                    match best {
                        None => best = Some(v),
                        Some(b) => {
                            if v.cmp_nulls_first(b).is_lt() {
                                best = Some(v);
                            }
                        }
                    }
                }
                Ok(best.cloned().unwrap_or(Value::Null))
            }),
        );
        register(
            "greatest",
            Arc::new(|args| {
                if args.is_empty() {
                    return Err(Error::TypeMismatch("greatest requires >=1 arg".into()));
                }
                let mut best: Option<&Value> = None;
                for v in args {
                    if v.is_null() {
                        continue;
                    }
                    match best {
                        None => best = Some(v),
                        Some(b) => {
                            if v.cmp_nulls_first(b).is_gt() {
                                best = Some(v);
                            }
                        }
                    }
                }
                Ok(best.cloned().unwrap_or(Value::Null))
            }),
        );
    }

    fn install_default_aggregates(&mut self) {
        self.aggregates.insert("count".into(), AggregateKind::Count);
        self.aggregates.insert("sum".into(), AggregateKind::Sum);
        self.aggregates.insert("avg".into(), AggregateKind::Avg);
        self.aggregates.insert("min".into(), AggregateKind::Min);
        self.aggregates.insert("max".into(), AggregateKind::Max);
    }
}

fn require_arity(name: &str, args: &[Value], n: usize) -> Result<()> {
    if args.len() != n {
        return Err(Error::Plan(format!(
            "{} expects {} arg(s); got {}",
            name,
            n,
            args.len()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_seeds_scalars_and_aggregates() {
        let r = FunctionRegistry::new();
        assert!(r.lookup_scalar("abs").is_some());
        assert!(r.lookup_scalar("ABS").is_some()); // case-insensitive
        assert_eq!(r.lookup_aggregate("count"), Some(AggregateKind::Count));
        assert_eq!(r.lookup_aggregate("AVG"), Some(AggregateKind::Avg));
    }

    #[test]
    fn abs_handles_int_float_and_null() {
        let r = FunctionRegistry::new();
        let f = r.lookup_scalar("abs").unwrap();
        assert_eq!((f.fun)(&[Value::Int64(-5)]).unwrap(), Value::Int64(5));
        assert_eq!(
            (f.fun)(&[Value::Float64(-1.5)]).unwrap(),
            Value::Float64(1.5)
        );
        assert_eq!((f.fun)(&[Value::Null]).unwrap(), Value::Null);
    }

    #[test]
    fn coalesce_returns_first_non_null() {
        let r = FunctionRegistry::new();
        let f = r.lookup_scalar("coalesce").unwrap();
        assert_eq!(
            (f.fun)(&[Value::Null, Value::Null, Value::Int64(7)]).unwrap(),
            Value::Int64(7),
        );
    }

    #[test]
    fn least_and_greatest_skip_null() {
        let r = FunctionRegistry::new();
        let least = r.lookup_scalar("least").unwrap();
        let greatest = r.lookup_scalar("greatest").unwrap();
        assert_eq!(
            (least.fun)(&[
                Value::Int64(3),
                Value::Null,
                Value::Int64(1),
                Value::Int64(2)
            ])
            .unwrap(),
            Value::Int64(1),
        );
        assert_eq!(
            (greatest.fun)(&[Value::Int64(3), Value::Null, Value::Int64(1)]).unwrap(),
            Value::Int64(3),
        );
    }

    #[test]
    fn length_counts_chars() {
        let r = FunctionRegistry::new();
        let f = r.lookup_scalar("length").unwrap();
        assert_eq!(
            (f.fun)(&[Value::Utf8("abc".into())]).unwrap(),
            Value::Int64(3)
        );
    }

    #[test]
    fn concat_concats_with_skip_null() {
        let r = FunctionRegistry::new();
        let f = r.lookup_scalar("concat").unwrap();
        assert_eq!(
            (f.fun)(&[
                Value::Utf8("a".into()),
                Value::Null,
                Value::Utf8("b".into())
            ])
            .unwrap(),
            Value::Utf8("ab".to_string()),
        );
    }
}

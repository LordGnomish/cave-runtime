//! Expression evaluator — translates sqlparser AST expressions to PgValue.

use sqlparser::ast::{self as ast, BinaryOperator, CastKind, Expr, FunctionArguments, UnaryOperator, Value};
use std::collections::HashMap;
use crate::error::{Error, PgError, Result, SqlState};
use crate::types::{oid, PgValue};
use crate::functions;

/// Evaluation context for expressions — maps column name/alias → value.
#[derive(Debug, Clone, Default)]
pub struct EvalContext {
    /// Current row being evaluated.
    pub row: HashMap<String, PgValue>,
    /// Outer row (for correlated subqueries / lateral joins).
    pub outer: Option<Box<EvalContext>>,
    /// Parameter values ($1, $2, ...).
    pub params: Vec<PgValue>,
}

impl EvalContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_row(row: HashMap<String, PgValue>) -> Self {
        Self { row, ..Default::default() }
    }

    pub fn get(&self, name: &str) -> Option<&PgValue> {
        let name_lower = name.to_lowercase();
        self.row.get(&name_lower)
            .or_else(|| self.outer.as_ref().and_then(|o| o.get(&name_lower)))
    }

    pub fn set(&mut self, name: &str, val: PgValue) {
        self.row.insert(name.to_lowercase(), val);
    }

    /// Build context from column names + values.
    pub fn from_columns_and_values(cols: &[String], values: &[Option<PgValue>]) -> Self {
        let mut row = HashMap::new();
        for (col, val) in cols.iter().zip(values.iter()) {
            row.insert(col.to_lowercase(), val.clone().unwrap_or(PgValue::Null));
        }
        Self { row, ..Default::default() }
    }
}

/// Evaluate a sqlparser expression given a context.
pub fn eval_expr(expr: &Expr, ctx: &EvalContext) -> Result<PgValue> {
    match expr {
        // ── Literals ─────────────────────────────────────────────────────────
        Expr::Value(v) => eval_value(v),

        // ── Column reference ─────────────────────────────────────────────────
        Expr::Identifier(ident) => {
            let name = ident.value.to_lowercase();
            Ok(ctx.get(&name).cloned().unwrap_or(PgValue::Null))
        }
        Expr::CompoundIdentifier(parts) => {
            // table.column or schema.table.column
            let name = parts.last().map(|p| p.value.to_lowercase()).unwrap_or_default();
            // Try qualified name first: "table.column"
            let qualified = parts.iter().map(|p| p.value.to_lowercase()).collect::<Vec<_>>().join(".");
            Ok(ctx.get(&qualified)
                .or_else(|| ctx.get(&name))
                .cloned()
                .unwrap_or(PgValue::Null))
        }

        // ── Parameters ($1, $2, ...) ──────────────────────────────────────────
        Expr::Value(Value::Placeholder(s)) => {
            // "$1" → params[0]
            if let Some(n) = s.strip_prefix('$').and_then(|s| s.parse::<usize>().ok()) {
                Ok(ctx.params.get(n - 1).cloned().unwrap_or(PgValue::Null))
            } else {
                Ok(PgValue::Null)
            }
        }

        // ── Arithmetic / comparison ──────────────────────────────────────────
        Expr::BinaryOp { left, op, right } => {
            eval_binary_op(left, op, right, ctx)
        }
        Expr::UnaryOp { op, expr } => {
            eval_unary_op(op, expr, ctx)
        }

        // ── IS NULL / IS NOT NULL ─────────────────────────────────────────────
        Expr::IsNull(inner) => {
            Ok(PgValue::Bool(eval_expr(inner, ctx)?.is_null()))
        }
        Expr::IsNotNull(inner) => {
            Ok(PgValue::Bool(!eval_expr(inner, ctx)?.is_null()))
        }
        Expr::IsDistinctFrom(a, b) => {
            let av = eval_expr(a, ctx)?;
            let bv = eval_expr(b, ctx)?;
            // IS DISTINCT FROM returns true when values differ, including NULL comparisons
            let result = match (&av, &bv) {
                (PgValue::Null, PgValue::Null) => false,
                (PgValue::Null, _) | (_, PgValue::Null) => true,
                _ => av != bv,
            };
            Ok(PgValue::Bool(result))
        }
        Expr::IsNotDistinctFrom(a, b) => {
            let av = eval_expr(a, ctx)?;
            let bv = eval_expr(b, ctx)?;
            let result = match (&av, &bv) {
                (PgValue::Null, PgValue::Null) => true,
                (PgValue::Null, _) | (_, PgValue::Null) => false,
                _ => av == bv,
            };
            Ok(PgValue::Bool(result))
        }

        // ── BETWEEN ──────────────────────────────────────────────────────────
        Expr::Between { expr: e, low, high, negated } => {
            let v = eval_expr(e, ctx)?;
            let lo = eval_expr(low, ctx)?;
            let hi = eval_expr(high, ctx)?;
            if v.is_null() || lo.is_null() || hi.is_null() { return Ok(PgValue::Null); }
            let ge_lo = v.compare(&lo).map(|o| o != std::cmp::Ordering::Less).unwrap_or(false);
            let le_hi = v.compare(&hi).map(|o| o != std::cmp::Ordering::Greater).unwrap_or(false);
            let result = ge_lo && le_hi;
            Ok(PgValue::Bool(if *negated { !result } else { result }))
        }

        // ── IN list ──────────────────────────────────────────────────────────
        Expr::InList { expr: e, list, negated } => {
            let v = eval_expr(e, ctx)?;
            if v.is_null() { return Ok(PgValue::Null); }
            let mut any_null = false;
            for item in list {
                let iv = eval_expr(item, ctx)?;
                if iv.is_null() { any_null = true; continue; }
                if v == iv { return Ok(PgValue::Bool(!negated)); }
            }
            if any_null { Ok(PgValue::Null) } else { Ok(PgValue::Bool(*negated)) }
        }

        // ── LIKE / ILIKE ─────────────────────────────────────────────────────
        Expr::Like { expr: e, pattern, negated, .. } => {
            let v = eval_expr(e, ctx)?;
            let p = eval_expr(pattern, ctx)?;
            if v.is_null() || p.is_null() { return Ok(PgValue::Null); }
            let s = v.to_text();
            let pat = p.to_text();
            let matches = like_match(&s, &pat, false);
            Ok(PgValue::Bool(if *negated { !matches } else { matches }))
        }
        Expr::ILike { expr: e, pattern, negated, .. } => {
            let v = eval_expr(e, ctx)?;
            let p = eval_expr(pattern, ctx)?;
            if v.is_null() || p.is_null() { return Ok(PgValue::Null); }
            let matches = like_match(&v.to_text(), &p.to_text(), true);
            Ok(PgValue::Bool(if *negated { !matches } else { matches }))
        }
        Expr::SimilarTo { expr: e, pattern, negated, .. } => {
            let v = eval_expr(e, ctx)?;
            let p = eval_expr(pattern, ctx)?;
            if v.is_null() || p.is_null() { return Ok(PgValue::Null); }
            // SIMILAR TO uses SQL regex (subset of POSIX)
            let re_pat = similar_to_regex(&p.to_text());
            let matches = regex::Regex::new(&re_pat)
                .map(|re| re.is_match(&v.to_text()))
                .unwrap_or(false);
            Ok(PgValue::Bool(if *negated { !matches } else { matches }))
        }

        // ── CASE WHEN ────────────────────────────────────────────────────────
        Expr::Case { operand, conditions, results, else_result } => {
            let op = match operand {
                Some(e) => Some(eval_expr(e, ctx)?),
                None => None,
            };
            for (cond, result) in conditions.iter().zip(results.iter()) {
                let cond_val = eval_expr(cond, ctx)?;
                let matches = match &op {
                    None => cond_val.is_true(),
                    Some(ov) => !cond_val.is_null() && cond_val == *ov,
                };
                if matches { return eval_expr(result, ctx); }
            }
            match else_result {
                Some(e) => eval_expr(e, ctx),
                None => Ok(PgValue::Null),
            }
        }

        // ── CAST ──────────────────────────────────────────────────────────────
        Expr::Cast { expr: e, data_type, kind, .. } => {
            let val = eval_expr(e, ctx)?;
            let type_str = data_type.to_string();
            let target_oid = crate::types::oid_for_type_name(&type_str).unwrap_or(oid::TEXT);
            match kind {
                CastKind::Cast | CastKind::DoubleColon => val.cast_to(target_oid),
                CastKind::TryCast | CastKind::SafeCast => Ok(val.cast_to(target_oid).unwrap_or(PgValue::Null)),
            }
        }

        // ── Function calls ────────────────────────────────────────────────────
        Expr::Function(f) => {
            let fn_name = f.name.to_string().to_lowercase();
            let mut arg_vals = Vec::new();
            if let FunctionArguments::List(arg_list) = &f.args {
                for arg in &arg_list.args {
                    match arg {
                        ast::FunctionArg::Named { arg, .. } | ast::FunctionArg::Unnamed(arg) => {
                            match arg {
                                ast::FunctionArgExpr::Expr(e) => {
                                    arg_vals.push(eval_expr(e, ctx)?);
                                }
                                ast::FunctionArgExpr::Wildcard => {
                                    // COUNT(*) handled by aggregator
                                    arg_vals.push(PgValue::Int4(1));
                                }
                                ast::FunctionArgExpr::QualifiedWildcard(_) => {
                                    arg_vals.push(PgValue::Int4(1));
                                }
                            }
                        }
                    }
                }
            }
            // Handle special forms
            match fn_name.as_str() {
                "nextval" => {
                    if let Some(PgValue::Text(seq_name)) = arg_vals.first() {
                        // Return a placeholder — actual sequence call happens in executor
                        return Ok(PgValue::Text(format!("__nextval__{seq_name}")));
                    }
                    Ok(PgValue::Null)
                }
                "currval" => {
                    if let Some(PgValue::Text(seq_name)) = arg_vals.first() {
                        return Ok(PgValue::Text(format!("__currval__{seq_name}")));
                    }
                    Ok(PgValue::Null)
                }
                _ => functions::call(&fn_name, arg_vals),
            }
        }

        // ── Subquery ─────────────────────────────────────────────────────────
        Expr::Subquery(_) => {
            // Subqueries are handled at query planning level
            Ok(PgValue::Null)
        }

        // ── EXISTS ───────────────────────────────────────────────────────────
        Expr::Exists { subquery: _, negated } => {
            Ok(PgValue::Bool(*negated)) // Simplified
        }

        // ── ANY / ALL ─────────────────────────────────────────────────────────
        Expr::AllOp { left, compare_op, right } | Expr::AnyOp { left, compare_op, right, .. } => {
            let v = eval_expr(left, ctx)?;
            let arr = eval_expr(right, ctx)?;
            let is_any = matches!(expr, Expr::AnyOp { .. });
            match arr {
                PgValue::Array { elements, .. } => {
                    let op = BinaryOperator::Eq; // compare_op is the actual op
                    let mut result = !is_any; // ANY: false until match; ALL: true until mismatch
                    for el in &elements {
                        let matches = match compare_op {
                            BinaryOperator::Eq => !v.is_null() && !el.is_null() && v == *el,
                            BinaryOperator::NotEq => !v.is_null() && !el.is_null() && v != *el,
                            BinaryOperator::Lt => v.compare(el) == Some(std::cmp::Ordering::Less),
                            BinaryOperator::LtEq => !matches!(v.compare(el), Some(std::cmp::Ordering::Greater)),
                            BinaryOperator::Gt => v.compare(el) == Some(std::cmp::Ordering::Greater),
                            BinaryOperator::GtEq => !matches!(v.compare(el), Some(std::cmp::Ordering::Less)),
                            _ => false,
                        };
                        if is_any && matches { result = true; break; }
                        if !is_any && !matches { result = false; break; }
                    }
                    Ok(PgValue::Bool(result))
                }
                _ => Ok(PgValue::Null),
            }
        }

        // ── Array constructor ─────────────────────────────────────────────────
        Expr::Array(arr) => {
            let elements: Vec<PgValue> = arr.elem.iter()
                .map(|e| eval_expr(e, ctx))
                .collect::<Result<_>>()?;
            let element_oid = elements.first().map(|v| v.oid()).unwrap_or(oid::TEXT);
            Ok(PgValue::Array { element_oid, elements })
        }

        // ── Array subscript: arr[n] ───────────────────────────────────────────
        Expr::Subscript { expr: e, subscript } => {
            let arr = eval_expr(e, ctx)?;
            let idx = match subscript.as_ref() {
                ast::Subscript::Index { index } => eval_expr(index, ctx)?.to_i64().unwrap_or(1) as usize,
                ast::Subscript::Slice { lower_bound, upper_bound, .. } => 1,
            };
            match arr {
                PgValue::Array { elements, .. } => {
                    if idx == 0 || idx > elements.len() {
                        Ok(PgValue::Null)
                    } else {
                        Ok(elements[idx - 1].clone())
                    }
                }
                PgValue::Jsonb(j) | PgValue::Json(j) => {
                    match j {
                        serde_json::Value::Array(arr) => {
                            Ok(arr.get(idx - 1).map(|v| PgValue::Jsonb(v.clone())).unwrap_or(PgValue::Null))
                        }
                        _ => Ok(PgValue::Null),
                    }
                }
                _ => Ok(PgValue::Null),
            }
        }

        // ── JSON operators ────────────────────────────────────────────────────
        Expr::JsonAccess { value, path } => {
            let v = eval_expr(value, ctx)?;
            eval_json_access(v, path)
        }

        // ── Row/Tuple constructor ─────────────────────────────────────────────
        Expr::Tuple(exprs) => {
            let vals: Vec<PgValue> = exprs.iter().map(|e| eval_expr(e, ctx)).collect::<Result<_>>()?;
            Ok(PgValue::Record(vals))
        }

        // ── Type-annotated string (e.g., TIMESTAMP '2024-01-01') ───────────────
        Expr::TypedString { data_type, value } => {
            let type_str = data_type.to_string();
            let target_oid = crate::types::oid_for_type_name(&type_str).unwrap_or(oid::TEXT);
            PgValue::Text(value.clone()).cast_to(target_oid)
        }

        // ── Named constant ────────────────────────────────────────────────────
        Expr::Interval(iv) => {
            // Parse interval literal
            let inner_val = eval_expr(&iv.value, ctx)?;
            let s = match inner_val {
                PgValue::Text(ref t) | PgValue::Varchar(ref t) => t.clone(),
                other => other.to_text(),
            };
            crate::functions::cast_to_interval(vec![PgValue::Text(s)])
        }

        Expr::Extract { field, expr: e, .. } => {
            let val = eval_expr(e, ctx)?;
            let field_name = field.to_string().to_lowercase();
            crate::functions::datetime::date_part(vec![PgValue::Text(field_name), val])
        }

        Expr::Position { expr: e, r#in } => {
            let substr = eval_expr(e, ctx)?;
            let haystack = eval_expr(r#in, ctx)?;
            functions::call("position", vec![substr, haystack])
        }

        Expr::Substring { expr: e, substring_from, substring_for, .. } => {
            let mut args = vec![eval_expr(e, ctx)?];
            if let Some(from) = substring_from {
                args.push(eval_expr(from, ctx)?);
            }
            if let Some(len) = substring_for {
                args.push(eval_expr(len, ctx)?);
            }
            functions::call("substring", args)
        }

        Expr::Trim { expr: e, trim_where, trim_what, .. } => {
            let s = eval_expr(e, ctx)?;
            let what = trim_what.as_ref().map(|w| eval_expr(w, ctx)).transpose()?;
            let fn_name = match trim_where {
                Some(ast::TrimWhereField::Leading) => "ltrim",
                Some(ast::TrimWhereField::Trailing) => "rtrim",
                _ => "btrim",
            };
            let mut args = vec![s];
            if let Some(w) = what { args.push(w); }
            functions::call(fn_name, args)
        }

        Expr::Overlay { expr: e, overlay_what, overlay_from, overlay_for } => {
            let s = eval_expr(e, ctx)?;
            let what = eval_expr(overlay_what, ctx)?;
            let from = eval_expr(overlay_from, ctx)?;
            let mut args = vec![s, what, from];
            if let Some(len) = overlay_for {
                args.push(eval_expr(len, ctx)?);
            }
            functions::call("overlay", args)
        }

        Expr::Collate { expr: e, .. } => eval_expr(e, ctx),

        Expr::Nested(inner) => eval_expr(inner, ctx),

        Expr::InSubquery { expr: _, subquery: _, negated } => {
            // Simplified — subquery handling in query module
            Ok(PgValue::Bool(*negated))
        }

        Expr::AtTimeZone { timestamp, time_zone } => {
            let ts = eval_expr(timestamp, ctx)?;
            let tz = eval_expr(time_zone, ctx)?;
            functions::datetime::timezone(vec![tz, ts])
        }

        Expr::Wildcard | Expr::QualifiedWildcard(..) => {
            Ok(PgValue::Null) // Should be expanded before reaching here
        }

        _ => Err(Error::Pg(PgError::feature_not_supported(&format!("expression: {expr}")))),
    }
}

fn eval_value(v: &Value) -> Result<PgValue> {
    Ok(match v {
        Value::Number(n, _) => {
            if n.contains('.') || n.contains('e') || n.contains('E') {
                n.parse::<f64>().map(PgValue::Float8)
                    .unwrap_or_else(|_| PgValue::Text(n.clone()))
            } else if let Ok(i) = n.parse::<i32>() {
                PgValue::Int4(i)
            } else if let Ok(i) = n.parse::<i64>() {
                PgValue::Int8(i)
            } else {
                n.parse::<rust_decimal::Decimal>().map(PgValue::Numeric)
                    .unwrap_or_else(|_| PgValue::Text(n.clone()))
            }
        }
        Value::SingleQuotedString(s) | Value::DoubleQuotedString(s) => PgValue::Text(s.clone()),
        Value::EscapedStringLiteral(s) => PgValue::Text(s.clone()),
        Value::NationalStringLiteral(s) => PgValue::Text(s.clone()),
        Value::HexStringLiteral(s) => {
            hex::decode(s).map(PgValue::Bytea).unwrap_or_else(|_| PgValue::Text(s.clone()))
        }
        Value::Boolean(b) => PgValue::Bool(*b),
        Value::Null => PgValue::Null,
        Value::Placeholder(s) => {
            // $1, $2, ... — return null here, params handled by caller
            PgValue::Null
        }
        _ => PgValue::Null,
    })
}

fn eval_binary_op(
    left: &Expr,
    op: &BinaryOperator,
    right: &Expr,
    ctx: &EvalContext,
) -> Result<PgValue> {
    // Handle string concatenation || special case
    if let BinaryOperator::StringConcat = op {
        let l = eval_expr(left, ctx)?;
        let r = eval_expr(right, ctx)?;
        if l.is_null() || r.is_null() { return Ok(PgValue::Null); }
        return Ok(PgValue::Text(l.to_text() + &r.to_text()));
    }

    // Handle JSONB operators
    if let BinaryOperator::Arrow | BinaryOperator::LongArrow = op {
        let l = eval_expr(left, ctx)?;
        let r = eval_expr(right, ctx)?;
        let key = r.to_text();
        return match l {
            PgValue::Json(j) | PgValue::Jsonb(j) => {
                let result = match &j {
                    serde_json::Value::Object(m) => m.get(&key).cloned(),
                    serde_json::Value::Array(a) => {
                        key.parse::<usize>().ok().and_then(|i| a.get(i)).cloned()
                    }
                    _ => None,
                };
                let val = result.map(|v| {
                    if matches!(op, BinaryOperator::LongArrow) {
                        // ->> returns text
                        match v { serde_json::Value::String(s) => PgValue::Text(s), other => PgValue::Text(other.to_string()) }
                    } else {
                        PgValue::Jsonb(v)
                    }
                }).unwrap_or(PgValue::Null);
                Ok(val)
            }
            _ => Ok(PgValue::Null),
        };
    }

    let lv = eval_expr(left, ctx)?;
    let rv = eval_expr(right, ctx)?;

    // NULL propagation for most operators
    match op {
        BinaryOperator::And => {
            return Ok(match (&lv, &rv) {
                (PgValue::Bool(false), _) | (_, PgValue::Bool(false)) => PgValue::Bool(false),
                (PgValue::Null, _) | (_, PgValue::Null) => PgValue::Null,
                (PgValue::Bool(a), PgValue::Bool(b)) => PgValue::Bool(*a && *b),
                _ => PgValue::Null,
            });
        }
        BinaryOperator::Or => {
            return Ok(match (&lv, &rv) {
                (PgValue::Bool(true), _) | (_, PgValue::Bool(true)) => PgValue::Bool(true),
                (PgValue::Null, _) | (_, PgValue::Null) => PgValue::Null,
                (PgValue::Bool(a), PgValue::Bool(b)) => PgValue::Bool(*a || *b),
                _ => PgValue::Null,
            });
        }
        BinaryOperator::Xor => {
            return Ok(match (&lv, &rv) {
                (PgValue::Bool(a), PgValue::Bool(b)) => PgValue::Bool(*a ^ *b),
                _ => PgValue::Null,
            });
        }
        _ => {}
    }

    if lv.is_null() || rv.is_null() {
        return Ok(PgValue::Null);
    }

    Ok(match op {
        BinaryOperator::Plus => numeric_binop(&lv, &rv, |a, b| a + b, |a, b| a + b)?,
        BinaryOperator::Minus => numeric_binop(&lv, &rv, |a, b| a - b, |a, b| a - b)?,
        BinaryOperator::Multiply => numeric_binop(&lv, &rv, |a, b| a * b, |a, b| a * b)?,
        BinaryOperator::Divide => {
            let b = rv.to_f64().unwrap_or(1.0);
            if b == 0.0 { return Err(Error::Pg(PgError::division_by_zero())); }
            numeric_binop(&lv, &rv, |a, b| a / b, |a, b| a / b)?
        }
        BinaryOperator::Modulo => {
            let b = rv.to_f64().unwrap_or(1.0);
            if b == 0.0 { return Err(Error::Pg(PgError::division_by_zero())); }
            numeric_binop(&lv, &rv, |a, b| a % b, |a, b| a % b)?
        }
        BinaryOperator::Eq => PgValue::Bool(lv == rv),
        BinaryOperator::NotEq => PgValue::Bool(lv != rv),
        BinaryOperator::Lt => PgValue::Bool(lv.compare(&rv) == Some(std::cmp::Ordering::Less)),
        BinaryOperator::LtEq => PgValue::Bool(!matches!(lv.compare(&rv), Some(std::cmp::Ordering::Greater))),
        BinaryOperator::Gt => PgValue::Bool(lv.compare(&rv) == Some(std::cmp::Ordering::Greater)),
        BinaryOperator::GtEq => PgValue::Bool(!matches!(lv.compare(&rv), Some(std::cmp::Ordering::Less))),
        BinaryOperator::PGRegexMatch => {
            let re = regex::Regex::new(&rv.to_text())
                .map_err(|e| Error::Pg(PgError::error(SqlState::INVALID_REGULAR_EXPRESSION, e.to_string())))?;
            PgValue::Bool(re.is_match(&lv.to_text()))
        }
        BinaryOperator::PGRegexIMatch => {
            let re = regex::RegexBuilder::new(&rv.to_text()).case_insensitive(true).build()
                .map_err(|e| Error::Pg(PgError::error(SqlState::INVALID_REGULAR_EXPRESSION, e.to_string())))?;
            PgValue::Bool(re.is_match(&lv.to_text()))
        }
        BinaryOperator::PGRegexNotMatch => {
            let re = regex::Regex::new(&rv.to_text())
                .map_err(|e| Error::Pg(PgError::error(SqlState::INVALID_REGULAR_EXPRESSION, e.to_string())))?;
            PgValue::Bool(!re.is_match(&lv.to_text()))
        }
        BinaryOperator::PGRegexNotIMatch => {
            let re = regex::RegexBuilder::new(&rv.to_text()).case_insensitive(true).build()
                .map_err(|e| Error::Pg(PgError::error(SqlState::INVALID_REGULAR_EXPRESSION, e.to_string())))?;
            PgValue::Bool(!re.is_match(&lv.to_text()))
        }
        BinaryOperator::PGCustomBinaryOperator(ops) => {
            let op_name = ops.join("");
            match op_name.as_str() {
                "@>" => PgValue::Bool(crate::functions::array::contains(&lv, &rv)
                    || crate::functions::json::jsonb_contains(&lv, &rv)),
                "<@" => PgValue::Bool(crate::functions::array::contains(&rv, &lv)
                    || crate::functions::json::jsonb_contains(&rv, &lv)),
                "&&" => PgValue::Bool(crate::functions::array::overlaps(&lv, &rv)),
                "?" => PgValue::Bool(crate::functions::json::jsonb_key_exists(&lv, &rv.to_text())),
                _ => PgValue::Null,
            }
        }
        BinaryOperator::BitwiseAnd => {
            let a = lv.to_i64().unwrap_or(0);
            let b = rv.to_i64().unwrap_or(0);
            PgValue::Int8(a & b)
        }
        BinaryOperator::BitwiseOr => {
            let a = lv.to_i64().unwrap_or(0);
            let b = rv.to_i64().unwrap_or(0);
            PgValue::Int8(a | b)
        }
        BinaryOperator::BitwiseXor => {
            let a = lv.to_i64().unwrap_or(0);
            let b = rv.to_i64().unwrap_or(0);
            PgValue::Int8(a ^ b)
        }
        BinaryOperator::PGBitwiseShiftLeft => {
            let a = lv.to_i64().unwrap_or(0);
            let b = rv.to_i64().unwrap_or(0);
            PgValue::Int8(a << (b as u32 % 64))
        }
        BinaryOperator::PGBitwiseShiftRight => {
            let a = lv.to_i64().unwrap_or(0);
            let b = rv.to_i64().unwrap_or(0);
            PgValue::Int8(a >> (b as u32 % 64))
        }
        _ => return Err(Error::Pg(PgError::feature_not_supported(&format!("operator {op}")))),
    })
}

fn eval_unary_op(op: &UnaryOperator, expr: &Expr, ctx: &EvalContext) -> Result<PgValue> {
    let v = eval_expr(expr, ctx)?;
    if v.is_null() { return Ok(PgValue::Null); }
    Ok(match op {
        UnaryOperator::Minus => match v {
            PgValue::Int2(n) => PgValue::Int2(-n),
            PgValue::Int4(n) => PgValue::Int4(-n),
            PgValue::Int8(n) => PgValue::Int8(-n),
            PgValue::Float4(n) => PgValue::Float4(-n),
            PgValue::Float8(n) => PgValue::Float8(-n),
            PgValue::Numeric(n) => PgValue::Numeric(-n),
            _ => return Err(Error::Pg(PgError::error(SqlState::DATATYPE_MISMATCH, "unary minus requires numeric"))),
        },
        UnaryOperator::Plus => v,
        UnaryOperator::Not => PgValue::Bool(!v.is_true()),
        UnaryOperator::PGBitwiseNot => {
            let n = v.to_i64().unwrap_or(0);
            PgValue::Int8(!n)
        }
        UnaryOperator::PGSquareRoot => {
            let f = v.to_f64().unwrap_or(0.0);
            if f < 0.0 { return Err(Error::Pg(PgError::error(SqlState::INVALID_ARGUMENT_FOR_LOGARITHM, "square root of negative"))); }
            PgValue::Float8(f.sqrt())
        }
        UnaryOperator::PGCubeRoot => PgValue::Float8(v.to_f64().unwrap_or(0.0).cbrt()),
        UnaryOperator::PGPostfixFactorial | UnaryOperator::PGPrefixFactorial => {
            crate::functions::math::factorial(vec![v])?
        }
        UnaryOperator::PGAbs => {
            crate::functions::math::abs(vec![v])?
        }
        _ => return Err(Error::Pg(PgError::feature_not_supported(&format!("unary op {op}")))),
    })
}

fn numeric_binop(
    a: &PgValue,
    b: &PgValue,
    int_op: impl Fn(i64, i64) -> i64,
    float_op: impl Fn(f64, f64) -> f64,
) -> Result<PgValue> {
    match (a, b) {
        (PgValue::Int2(x), PgValue::Int2(y)) => Ok(PgValue::Int2(int_op(*x as i64, *y as i64) as i16)),
        (PgValue::Int4(x), PgValue::Int4(y)) => Ok(PgValue::Int4(int_op(*x as i64, *y as i64) as i32)),
        (PgValue::Int8(x), PgValue::Int8(y)) => Ok(PgValue::Int8(int_op(*x, *y))),
        (PgValue::Float4(x), PgValue::Float4(y)) => Ok(PgValue::Float4(float_op(*x as f64, *y as f64) as f32)),
        (PgValue::Float8(x), PgValue::Float8(y)) => Ok(PgValue::Float8(float_op(*x, *y))),
        _ => {
            // Mixed types: coerce to float8
            let af = a.to_f64().ok_or_else(|| Error::Pg(PgError::error(
                SqlState::DATATYPE_MISMATCH, format!("cannot apply arithmetic to {}", crate::types::type_name_for_oid(a.oid())))))?;
            let bf = b.to_f64().ok_or_else(|| Error::Pg(PgError::error(
                SqlState::DATATYPE_MISMATCH, format!("cannot apply arithmetic to {}", crate::types::type_name_for_oid(b.oid())))))?;
            Ok(PgValue::Float8(float_op(af, bf)))
        }
    }
}

fn eval_json_access(v: PgValue, path: &ast::JsonPath) -> Result<PgValue> {
    let mut current = match v {
        PgValue::Json(j) | PgValue::Jsonb(j) => j,
        _ => return Ok(PgValue::Null),
    };
    let ctx = EvalContext::new();
    for elem in &path.path {
        current = match elem {
            ast::JsonPathElem::Dot { key, .. } => {
                match &current {
                    serde_json::Value::Object(m) => m.get(key.as_str()).cloned().unwrap_or(serde_json::Value::Null),
                    _ => serde_json::Value::Null,
                }
            }
            ast::JsonPathElem::Bracket { key } => {
                match eval_expr(key, &ctx)? {
                    PgValue::Text(k) => match &current {
                        serde_json::Value::Object(m) => m.get(&k).cloned().unwrap_or(serde_json::Value::Null),
                        _ => serde_json::Value::Null,
                    },
                    PgValue::Int4(i) => match &current {
                        serde_json::Value::Array(a) => a.get(i as usize).cloned().unwrap_or(serde_json::Value::Null),
                        _ => serde_json::Value::Null,
                    },
                    PgValue::Int8(i) => match &current {
                        serde_json::Value::Array(a) => a.get(i as usize).cloned().unwrap_or(serde_json::Value::Null),
                        _ => serde_json::Value::Null,
                    },
                    _ => serde_json::Value::Null,
                }
            }
        };
    }
    if current.is_null() { Ok(PgValue::Null) } else { Ok(PgValue::Jsonb(current)) }
}

/// PostgreSQL LIKE pattern matching.
pub fn like_match(s: &str, pattern: &str, case_insensitive: bool) -> bool {
    let s_chars: Vec<char> = if case_insensitive { s.to_lowercase().chars().collect() } else { s.chars().collect() };
    let p_chars: Vec<char> = if case_insensitive { pattern.to_lowercase().chars().collect() } else { pattern.chars().collect() };
    like_match_chars(&s_chars, &p_chars)
}

fn like_match_chars(s: &[char], p: &[char]) -> bool {
    let (mut si, mut pi) = (0usize, 0usize);
    let (mut star_si, mut star_pi) = (usize::MAX, usize::MAX);
    while si < s.len() {
        if pi < p.len() && (p[pi] == '_' || p[pi] == s[si]) {
            si += 1; pi += 1;
        } else if pi < p.len() && p[pi] == '%' {
            star_si = si; star_pi = pi;
            pi += 1;
        } else if star_pi != usize::MAX {
            star_si += 1;
            si = star_si; pi = star_pi + 1;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '%' { pi += 1; }
    pi == p.len()
}

/// Convert SQL SIMILAR TO pattern to a POSIX regex.
fn similar_to_regex(pattern: &str) -> String {
    let mut result = String::from("^");
    for c in pattern.chars() {
        match c {
            '%' => result.push_str(".*"),
            '_' => result.push('.'),
            '.' | '*' | '+' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '^' | '$' | '\\' | '|' => {
                result.push('\\');
                result.push(c);
            }
            _ => result.push(c),
        }
    }
    result.push('$');
    result
}

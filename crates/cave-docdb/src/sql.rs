// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! MongoDB → PostgreSQL SQL translation (FerretDB-style clean-room).
//!
//! This is the heart of the cave-docdb *hybrid* strategy: the MongoDB
//! wire-protocol surface (OP_MSG / BSON, see [`crate::wire`]) is paired with a
//! PostgreSQL storage backend by translating Mongo query documents and
//! aggregation pipelines into parameterised SQL over a JSONB document column.
//!
//! Storage model (matches FerretDB v1 `internal/handlers/pg`): every collection
//! is a table with a single `_jsonb jsonb` column holding the document. Query
//! operators become expressions over `_jsonb` using PostgreSQL's native JSONB
//! operators (`->`, `->>`, `#>`, `?`, `@>`, `jsonb_array_length`, ...).
//!
//! All emitted SQL is **parameterised** (`$1`, `$2`, ...) — user values never
//! appear inline, which is both injection-safe and lets the driver bind types.
//! Field names are SQL-string-literal escaped; table identifiers are quoted.

use serde_json::Value;

/// A `WHERE` clause fragment plus its ordered bind parameters.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WhereClause {
    /// SQL boolean expression (no leading `WHERE`). Empty string == match-all.
    pub sql: String,
    /// Ordered bind parameters, textual; `$N` placeholders index into this.
    pub params: Vec<String>,
}

/// Mutable translation context accumulating ordered bind parameters.
///
/// Shared across nested predicate translation so `$N` placeholders stay
/// globally unique and in source order, matching libpq numbered parameters.
#[derive(Default)]
struct Ctx {
    params: Vec<String>,
}

impl Ctx {
    /// Register a bind value and return its `$N` placeholder.
    fn bind(&mut self, value: impl Into<String>) -> String {
        self.params.push(value.into());
        format!("${}", self.params.len())
    }
}

/// Escape a single SQL string-literal segment (`'` → `''`).
fn esc(s: &str) -> String {
    s.replace('\'', "''")
}

/// JSONB extraction expression yielding a `jsonb` value for a (possibly
/// dotted) field path: `_jsonb -> 'a'` or `_jsonb #> '{a,b}'`.
fn path_value(field: &str) -> String {
    if let Some((segs, _)) = split_path(field) {
        format!("_jsonb #> '{{{}}}'", segs.join(","))
    } else {
        format!("_jsonb -> '{}'", esc(field))
    }
}

/// JSONB extraction expression yielding `text` for a field path:
/// `_jsonb ->> 'a'` or `_jsonb #>> '{a,b}'`.
fn path_text(field: &str) -> String {
    if let Some((segs, _)) = split_path(field) {
        format!("_jsonb #>> '{{{}}}'", segs.join(","))
    } else {
        format!("_jsonb ->> '{}'", esc(field))
    }
}

/// Key-presence test for a field path.
fn path_exists(field: &str) -> String {
    if let Some((segs, _)) = split_path(field) {
        format!("(_jsonb #> '{{{}}}') IS NOT NULL", segs.join(","))
    } else {
        format!("_jsonb ? '{}'", esc(field))
    }
}

/// Split a dotted path into escaped segments; `None` for a single segment.
fn split_path(field: &str) -> Option<(Vec<String>, ())> {
    if field.contains('.') {
        Some((field.split('.').map(esc).collect(), ()))
    } else {
        None
    }
}

/// Compact JSON encoding of a value for a `::jsonb` bind parameter.
fn json_param(v: &Value) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "null".to_string())
}

/// Translate a MongoDB filter document into a parameterised SQL `WHERE` clause.
///
/// Empty `sql` means "match all rows". The returned `params` are positional and
/// must be bound in order to the `$1..$N` placeholders.
pub fn filter_to_where(filter: &Value) -> WhereClause {
    let mut ctx = Ctx::default();
    let sql = translate_filter(filter, &mut ctx);
    WhereClause {
        sql,
        params: ctx.params,
    }
}

/// Translate a filter object into a boolean SQL expression.
fn translate_filter(filter: &Value, ctx: &mut Ctx) -> String {
    let Some(obj) = filter.as_object() else {
        return String::new();
    };
    let mut terms: Vec<String> = Vec::new();
    // Deterministic order: serde_json maps are sorted, but be explicit.
    let mut keys: Vec<&String> = obj.keys().collect();
    keys.sort();
    for key in keys {
        let value = &obj[key];
        match key.as_str() {
            "$and" => {
                if let Some(t) = combine_logical(value, " AND ", ctx, false) {
                    terms.push(t);
                }
            }
            "$or" => {
                if let Some(t) = combine_logical(value, " OR ", ctx, true) {
                    terms.push(t);
                }
            }
            "$nor" => {
                if let Some(t) = combine_logical(value, " OR ", ctx, true) {
                    terms.push(format!("NOT {}", t));
                }
            }
            _ => terms.push(translate_field(key, value, ctx)),
        }
    }
    terms.join(" AND ")
}

/// Combine a `$and`/`$or`/`$nor` array of sub-filters.
fn combine_logical(value: &Value, joiner: &str, ctx: &mut Ctx, parenthesize: bool) -> Option<String> {
    let arr = value.as_array()?;
    let parts: Vec<String> = arr
        .iter()
        .map(|sub| translate_filter(sub, ctx))
        .filter(|s| !s.is_empty())
        .collect();
    if parts.is_empty() {
        return None;
    }
    if parenthesize || parts.len() > 1 {
        Some(format!("({})", parts.join(joiner)))
    } else {
        Some(parts.join(joiner))
    }
}

/// Translate a single `field: spec` predicate.
fn translate_field(field: &str, spec: &Value, ctx: &mut Ctx) -> String {
    // An object whose keys all start with `$` is an operator spec; otherwise
    // (and for any non-object) it is an equality match against the literal.
    if let Some(obj) = spec.as_object() {
        if !obj.is_empty() && obj.keys().all(|k| k.starts_with('$')) {
            let mut keys: Vec<&String> = obj.keys().collect();
            keys.sort();
            let mut parts: Vec<String> = Vec::new();
            for k in keys {
                // $options is folded into its sibling $regex below.
                if k == "$options" {
                    continue;
                }
                if k == "$regex" {
                    let options = obj.get("$options").and_then(|o| o.as_str()).unwrap_or("");
                    parts.push(translate_regex(field, &obj[k], options, ctx));
                    continue;
                }
                parts.push(translate_operator(field, k, &obj[k], ctx));
            }
            return parts.join(" AND ");
        }
    }
    let p = ctx.bind(json_param(spec));
    format!("{} = {}::jsonb", path_value(field), p)
}

/// Translate one `$operator` against a field.
fn translate_operator(field: &str, op: &str, arg: &Value, ctx: &mut Ctx) -> String {
    let cmp = |sym: &str, ctx: &mut Ctx| {
        let p = ctx.bind(json_param(arg));
        format!("{} {} {}::jsonb", path_value(field), sym, p)
    };
    match op {
        "$eq" => cmp("=", ctx),
        "$gt" => cmp(">", ctx),
        "$gte" => cmp(">=", ctx),
        "$lt" => cmp("<", ctx),
        "$lte" => cmp("<=", ctx),
        "$ne" => {
            let p = ctx.bind(json_param(arg));
            format!("{} IS DISTINCT FROM {}::jsonb", path_value(field), p)
        }
        "$in" => in_clause(field, arg, ctx, false),
        "$nin" => in_clause(field, arg, ctx, true),
        "$exists" => {
            let present = arg.as_bool().unwrap_or(true);
            if present {
                path_exists(field)
            } else {
                format!("NOT ({})", path_exists(field))
            }
        }
        "$regex" => translate_regex(field, arg, "", ctx),
        "$options" => "TRUE".to_string(),
        "$size" => {
            let p = ctx.bind(json_param(arg));
            format!("jsonb_array_length({}) = {}::int", path_value(field), p)
        }
        "$all" => {
            let p = ctx.bind(json_param(arg));
            format!("{} @> {}::jsonb", path_value(field), p)
        }
        "$mod" => {
            let empty = Vec::new();
            let parts = arg.as_array().unwrap_or(&empty);
            let divisor = parts.first().cloned().unwrap_or(Value::from(1));
            let remainder = parts.get(1).cloned().unwrap_or(Value::from(0));
            let d = ctx.bind(json_param(&divisor));
            let r = ctx.bind(json_param(&remainder));
            format!(
                "(({})::numeric % {}::numeric) = {}::numeric",
                path_text(field),
                d,
                r
            )
        }
        "$not" => format!("NOT ({})", translate_field(field, arg, ctx)),
        _ => {
            // Unknown operator: fall back to equality against the raw spec so
            // we never silently drop a predicate (over-restrict, never leak).
            let p = ctx.bind(json_param(arg));
            format!("{} = {}::jsonb", path_value(field), p)
        }
    }
}

/// `$regex` translation. POSIX `~`, or `~*` when the `i` option is set.
fn translate_regex(field: &str, arg: &Value, options: &str, ctx: &mut Ctx) -> String {
    let pattern = arg.as_str().unwrap_or("").to_string();
    let p = ctx.bind(pattern);
    let op = if options.contains('i') { "~*" } else { "~" };
    format!("{} {} {}", path_text(field), op, p)
}

/// `$in` / `$nin` translation, missing-field aware for `$nin`.
fn in_clause(field: &str, arg: &Value, ctx: &mut Ctx, negate: bool) -> String {
    let empty = Vec::new();
    let items = arg.as_array().unwrap_or(&empty);
    if items.is_empty() {
        // `$in []` matches nothing; `$nin []` matches everything.
        return if negate { "TRUE".into() } else { "FALSE".into() };
    }
    let placeholders: Vec<String> = items
        .iter()
        .map(|v| format!("{}::jsonb", ctx.bind(json_param(v))))
        .collect();
    let list = placeholders.join(", ");
    if negate {
        format!(
            "(NOT ({}) OR {} NOT IN ({}))",
            path_exists(field),
            path_value(field),
            list
        )
    } else {
        format!("{} IN ({})", path_value(field), list)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn wc(filter: serde_json::Value) -> WhereClause {
        filter_to_where(&filter)
    }

    #[test]
    fn eq_scalar_becomes_jsonb_equality() {
        let w = wc(json!({"status": "active"}));
        assert_eq!(w.sql, "_jsonb -> 'status' = $1::jsonb");
        assert_eq!(w.params, vec!["\"active\""]);
    }

    #[test]
    fn comparison_operator_gt() {
        let w = wc(json!({"age": {"$gt": 21}}));
        assert_eq!(w.sql, "_jsonb -> 'age' > $1::jsonb");
        assert_eq!(w.params, vec!["21"]);
    }

    #[test]
    fn ne_uses_is_distinct_from() {
        let w = wc(json!({"s": {"$ne": "x"}}));
        assert_eq!(w.sql, "_jsonb -> 's' IS DISTINCT FROM $1::jsonb");
        assert_eq!(w.params, vec!["\"x\""]);
    }

    #[test]
    fn in_operator_expands_placeholders() {
        let w = wc(json!({"x": {"$in": [1, 2]}}));
        assert_eq!(w.sql, "_jsonb -> 'x' IN ($1::jsonb, $2::jsonb)");
        assert_eq!(w.params, vec!["1", "2"]);
    }

    #[test]
    fn nin_handles_missing_field() {
        let w = wc(json!({"x": {"$nin": [1, 2]}}));
        assert_eq!(
            w.sql,
            "(NOT (_jsonb ? 'x') OR _jsonb -> 'x' NOT IN ($1::jsonb, $2::jsonb))"
        );
        assert_eq!(w.params, vec!["1", "2"]);
    }

    #[test]
    fn exists_true_and_false() {
        assert_eq!(wc(json!({"e": {"$exists": true}})).sql, "_jsonb ? 'e'");
        assert_eq!(
            wc(json!({"e": {"$exists": false}})).sql,
            "NOT (_jsonb ? 'e')"
        );
    }

    #[test]
    fn implicit_and_sorts_fields_deterministically() {
        let w = wc(json!({"b": 2, "a": 1}));
        assert_eq!(
            w.sql,
            "_jsonb -> 'a' = $1::jsonb AND _jsonb -> 'b' = $2::jsonb"
        );
        assert_eq!(w.params, vec!["1", "2"]);
    }

    #[test]
    fn explicit_or() {
        let w = wc(json!({"$or": [{"a": 1}, {"b": 2}]}));
        assert_eq!(
            w.sql,
            "(_jsonb -> 'a' = $1::jsonb OR _jsonb -> 'b' = $2::jsonb)"
        );
    }

    #[test]
    fn nor_negates_or() {
        let w = wc(json!({"$nor": [{"a": 1}, {"b": 2}]}));
        assert_eq!(
            w.sql,
            "NOT (_jsonb -> 'a' = $1::jsonb OR _jsonb -> 'b' = $2::jsonb)"
        );
    }

    #[test]
    fn regex_with_case_insensitive_option() {
        let w = wc(json!({"n": {"$regex": "^a", "$options": "i"}}));
        assert_eq!(w.sql, "_jsonb ->> 'n' ~* $1");
        assert_eq!(w.params, vec!["^a"]);
    }

    #[test]
    fn dotted_path_uses_hash_arrow() {
        let w = wc(json!({"a.b": {"$gt": 5}}));
        assert_eq!(w.sql, "_jsonb #> '{a,b}' > $1::jsonb");
    }

    #[test]
    fn size_operator() {
        let w = wc(json!({"t": {"$size": 3}}));
        assert_eq!(w.sql, "jsonb_array_length(_jsonb -> 't') = $1::int");
        assert_eq!(w.params, vec!["3"]);
    }

    #[test]
    fn not_operator_negates_inner() {
        let w = wc(json!({"x": {"$not": {"$gt": 5}}}));
        assert_eq!(w.sql, "NOT (_jsonb -> 'x' > $1::jsonb)");
    }

    #[test]
    fn multiple_operators_on_one_field_anded() {
        let w = wc(json!({"age": {"$gt": 1, "$lt": 10}}));
        assert_eq!(
            w.sql,
            "_jsonb -> 'age' > $1::jsonb AND _jsonb -> 'age' < $2::jsonb"
        );
    }

    #[test]
    fn all_operator_uses_containment() {
        let w = wc(json!({"tags": {"$all": ["a", "b"]}}));
        assert_eq!(w.sql, "_jsonb -> 'tags' @> $1::jsonb");
        assert_eq!(w.params, vec!["[\"a\",\"b\"]"]);
    }

    #[test]
    fn mod_operator() {
        let w = wc(json!({"n": {"$mod": [4, 1]}}));
        assert_eq!(
            w.sql,
            "((_jsonb ->> 'n')::numeric % $1::numeric) = $2::numeric"
        );
        assert_eq!(w.params, vec!["4", "1"]);
    }

    #[test]
    fn empty_filter_is_match_all() {
        assert_eq!(wc(json!({})).sql, "");
    }
}

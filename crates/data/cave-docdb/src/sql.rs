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

/// A complete SQL statement plus its ordered bind parameters.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SqlQuery {
    /// The SQL text with `$1..$N` placeholders.
    pub sql: String,
    /// Ordered bind parameters.
    pub params: Vec<String>,
}

/// Quote a SQL identifier (table/collection name), escaping embedded `"`.
fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Append a `WHERE` clause (using the shared ctx) and return its SQL text.
fn where_suffix(filter: &Value, ctx: &mut Ctx) -> String {
    let body = translate_filter(filter, ctx);
    if body.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", body)
    }
}

/// Build the `SELECT` projection list for an inclusion projection.
///
/// `_id` is implicitly included unless explicitly set to `0`. Returns `None`
/// when the projection is empty or an exclusion projection (handled later in
/// the document layer over the full `_jsonb`).
fn projection_select(projection: Option<&Value>) -> Option<String> {
    let obj = projection?.as_object()?;
    if obj.is_empty() {
        return None;
    }
    // Inclusion projection iff every non-`_id` value is truthy.
    let is_inclusion = obj
        .iter()
        .filter(|(k, _)| *k != "_id")
        .all(|(_, v)| truthy(v));
    if !is_inclusion {
        return None;
    }
    let mut pairs: Vec<String> = Vec::new();
    let include_id = obj.get("_id").map(truthy).unwrap_or(true);
    if include_id {
        pairs.push(format!("'_id', {}", path_value("_id")));
    }
    let mut fields: Vec<&String> = obj.keys().filter(|k| *k != "_id").collect();
    fields.sort();
    for f in fields {
        pairs.push(format!("'{}', {}", esc(f), path_value(f)));
    }
    Some(format!("jsonb_build_object({}) AS _jsonb", pairs.join(", ")))
}

/// Truthiness of a projection flag (`1`, `true` → include).
fn truthy(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().map(|f| f != 0.0).unwrap_or(false),
        _ => false,
    }
}

/// Translate a `find` into a `SELECT` statement.
pub fn find_to_sql(
    table: &str,
    filter: &Value,
    projection: Option<&Value>,
    sort: Option<&Value>,
    skip: Option<i64>,
    limit: Option<i64>,
) -> SqlQuery {
    let mut ctx = Ctx::default();
    let select = projection_select(projection).unwrap_or_else(|| "_jsonb".to_string());
    let mut sql = format!("SELECT {} FROM {}", select, quote_ident(table));
    sql.push_str(&where_suffix(filter, &mut ctx));
    if let Some(s) = sort.and_then(|v| v.as_object()) {
        let order: Vec<String> = s
            .iter()
            .map(|(field, dir)| {
                let d = if dir.as_i64().unwrap_or(1) < 0 { "DESC" } else { "ASC" };
                format!("{} {}", path_value(field), d)
            })
            .collect();
        if !order.is_empty() {
            sql.push_str(&format!(" ORDER BY {}", order.join(", ")));
        }
    }
    if let Some(l) = limit {
        sql.push_str(&format!(" LIMIT {}", l));
    }
    if let Some(s) = skip {
        sql.push_str(&format!(" OFFSET {}", s));
    }
    SqlQuery {
        sql,
        params: ctx.params,
    }
}

/// Translate a single-document `insert` into an `INSERT` statement.
pub fn insert_to_sql(table: &str, doc: &Value) -> SqlQuery {
    let mut ctx = Ctx::default();
    let p = ctx.bind(json_param(doc));
    SqlQuery {
        sql: format!(
            "INSERT INTO {} (_jsonb) VALUES ({}::jsonb)",
            quote_ident(table),
            p
        ),
        params: ctx.params,
    }
}

/// Translate an `update` (replacement or `$set`/`$unset`/`$inc`) into `UPDATE`.
pub fn update_to_sql(table: &str, filter: &Value, update: &Value) -> SqlQuery {
    let mut ctx = Ctx::default();
    let assignment = build_assignment(update, &mut ctx);
    let mut sql = format!(
        "UPDATE {} SET _jsonb = {}",
        quote_ident(table),
        assignment
    );
    sql.push_str(&where_suffix(filter, &mut ctx));
    SqlQuery {
        sql,
        params: ctx.params,
    }
}

/// Build the `_jsonb` assignment expression for an update document.
fn build_assignment(update: &Value, ctx: &mut Ctx) -> String {
    let Some(obj) = update.as_object() else {
        let p = ctx.bind(json_param(update));
        return format!("{}::jsonb", p);
    };
    let has_ops = obj.keys().any(|k| k.starts_with('$'));
    if !has_ops {
        // Whole-document replacement.
        let p = ctx.bind(json_param(update));
        return format!("{}::jsonb", p);
    }
    let mut expr = "_jsonb".to_string();
    // Deterministic operator order: $inc, $set, $unset (alphabetical).
    let mut keys: Vec<&String> = obj.keys().collect();
    keys.sort();
    for key in keys {
        match key.as_str() {
            "$inc" => {
                if let Some(fields) = obj[key].as_object() {
                    let mut fs: Vec<&String> = fields.keys().collect();
                    fs.sort();
                    for f in fs {
                        let p = ctx.bind(json_param(&fields[f]));
                        expr = format!(
                            "jsonb_set({}, '{{{}}}', to_jsonb(COALESCE(({} ->> '{}')::numeric, 0) + {}::numeric))",
                            expr,
                            esc(f),
                            "_jsonb",
                            esc(f),
                            p
                        );
                    }
                }
            }
            "$set" => {
                let p = ctx.bind(json_param(&obj[key]));
                expr = format!("({}) || {}::jsonb", expr, p);
            }
            "$unset" => {
                if let Some(fields) = obj[key].as_object() {
                    let mut fs: Vec<&String> = fields.keys().collect();
                    fs.sort();
                    for f in fs {
                        expr = format!("{} - '{}'", expr, esc(f));
                    }
                }
            }
            _ => {}
        }
    }
    expr
}

/// Translate a `delete` into a `DELETE` statement.
pub fn delete_to_sql(table: &str, filter: &Value) -> SqlQuery {
    let mut ctx = Ctx::default();
    let mut sql = format!("DELETE FROM {}", quote_ident(table));
    sql.push_str(&where_suffix(filter, &mut ctx));
    SqlQuery {
        sql,
        params: ctx.params,
    }
}

/// Translate an aggregation pipeline into a single SQL statement.
///
/// Supports the common linear shapes: `$match` (folded into `WHERE`), `$sort`,
/// `$skip`, `$limit`, `$project` (inclusion), `$group` (with `$sum`/`$avg`/
/// `$min`/`$max`/`$count` accumulators and a `$field` or `null` `_id`), and
/// `$count`. Stages outside this set leave the pipeline to the in-memory
/// executor — callers should fall back when `pipeline_to_sql` cannot represent
/// a stage (signalled by `None`).
pub fn pipeline_to_sql(table: &str, pipeline: &[Value]) -> Option<SqlQuery> {
    let mut ctx = Ctx::default();
    let mut where_terms: Vec<String> = Vec::new();
    let mut sort: Option<Value> = None;
    let mut skip: Option<i64> = None;
    let mut limit: Option<i64> = None;
    let mut project: Option<Value> = None;
    let mut group: Option<Value> = None;
    let mut count_name: Option<String> = None;

    for stage in pipeline {
        let obj = stage.as_object()?;
        if obj.len() != 1 {
            return None;
        }
        let (name, body) = obj.iter().next()?;
        match name.as_str() {
            "$match" => {
                let term = translate_filter(body, &mut ctx);
                if !term.is_empty() {
                    where_terms.push(term);
                }
            }
            "$sort" => sort = Some(body.clone()),
            "$skip" => skip = body.as_i64(),
            "$limit" => limit = body.as_i64(),
            "$project" => project = Some(body.clone()),
            "$group" => group = Some(body.clone()),
            "$count" => count_name = body.as_str().map(|s| s.to_string()),
            // Any stage we cannot represent in SQL → defer to the engine.
            _ => return None,
        }
    }

    let from = quote_ident(table);
    let where_sql = if where_terms.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", where_terms.join(" AND "))
    };

    // $count is terminal.
    if let Some(name) = count_name {
        return Some(SqlQuery {
            sql: format!(
                "SELECT jsonb_build_object('{}', count(*)) AS _jsonb FROM {}{}",
                esc(&name),
                from,
                where_sql
            ),
            params: ctx.params,
        });
    }

    // $group is terminal (aside from a trailing $sort/$limit, omitted here).
    if let Some(g) = group {
        let (select, group_by) = group_to_sql(&g)?;
        let mut sql = format!("SELECT {} FROM {}{}", select, from, where_sql);
        if let Some(gb) = group_by {
            sql.push_str(&format!(" GROUP BY {}", gb));
        }
        return Some(SqlQuery {
            sql,
            params: ctx.params,
        });
    }

    // Linear projection pipeline.
    let select = projection_select(project.as_ref()).unwrap_or_else(|| "_jsonb".to_string());
    let mut sql = format!("SELECT {} FROM {}{}", select, from, where_sql);
    if let Some(s) = sort.as_ref().and_then(|v| v.as_object()) {
        let order: Vec<String> = s
            .iter()
            .map(|(field, dir)| {
                let d = if dir.as_i64().unwrap_or(1) < 0 { "DESC" } else { "ASC" };
                format!("{} {}", path_value(field), d)
            })
            .collect();
        if !order.is_empty() {
            sql.push_str(&format!(" ORDER BY {}", order.join(", ")));
        }
    }
    if let Some(l) = limit {
        sql.push_str(&format!(" LIMIT {}", l));
    }
    if let Some(s) = skip {
        sql.push_str(&format!(" OFFSET {}", s));
    }
    Some(SqlQuery {
        sql,
        params: ctx.params,
    })
}

/// Build a `$group` `SELECT` list and `GROUP BY` expression.
///
/// Returns `(select_list, Some(group_by_expr))`, or a `None` group-by for a
/// whole-collection aggregate (`_id: null`).
fn group_to_sql(spec: &Value) -> Option<(String, Option<String>)> {
    let obj = spec.as_object()?;
    let id = obj.get("_id")?;
    // _id expression and the GROUP BY key (None == aggregate over all rows).
    let (id_expr, group_by) = match id {
        Value::Null => ("'null'::jsonb".to_string(), None),
        Value::String(s) if s.starts_with('$') => {
            let e = path_value(s.trim_start_matches('$'));
            (e.clone(), Some(e))
        }
        other => {
            // Constant grouping key.
            (format!("'{}'::jsonb", esc(&json_param(other))), None)
        }
    };
    let mut pairs = vec![format!("'_id', {}", id_expr)];
    let mut fields: Vec<&String> = obj.keys().filter(|k| *k != "_id").collect();
    fields.sort();
    for f in fields {
        let acc = accumulator_to_sql(&obj[f])?;
        pairs.push(format!("'{}', {}", esc(f), acc));
    }
    Some((
        format!("jsonb_build_object({}) AS _jsonb", pairs.join(", ")),
        group_by,
    ))
}

/// Translate one accumulator expression (`{"$sum": "$f"}` etc.) into SQL.
fn accumulator_to_sql(spec: &Value) -> Option<String> {
    let obj = spec.as_object()?;
    let (op, arg) = obj.iter().next()?;
    // numeric field reference "$f" → cast to numeric; literal number → count.
    let numeric_arg = |arg: &Value| -> Option<String> {
        match arg {
            Value::String(s) if s.starts_with('$') => {
                Some(format!("({})::numeric", path_text(s.trim_start_matches('$'))))
            }
            _ => None,
        }
    };
    let sql = match op.as_str() {
        "$sum" => match numeric_arg(arg) {
            Some(col) => format!("to_jsonb(sum({}))", col),
            // $sum: <constant> counts matching rows.
            None => "to_jsonb(count(*))".to_string(),
        },
        "$avg" => format!("to_jsonb(avg({}))", numeric_arg(arg)?),
        "$min" => format!("to_jsonb(min({}))", numeric_arg(arg)?),
        "$max" => format!("to_jsonb(max({}))", numeric_arg(arg)?),
        "$count" => "to_jsonb(count(*))".to_string(),
        _ => return None,
    };
    Some(sql)
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

    // ── find_to_sql ──────────────────────────────────────────────────────

    #[test]
    fn find_basic_with_filter() {
        let q = find_to_sql("users", &json!({"age": {"$gt": 21}}), None, None, None, None);
        assert_eq!(
            q.sql,
            "SELECT _jsonb FROM \"users\" WHERE _jsonb -> 'age' > $1::jsonb"
        );
        assert_eq!(q.params, vec!["21"]);
    }

    #[test]
    fn find_match_all_no_where() {
        let q = find_to_sql("c", &json!({}), None, None, None, None);
        assert_eq!(q.sql, "SELECT _jsonb FROM \"c\"");
        assert!(q.params.is_empty());
    }

    #[test]
    fn find_sort_skip_limit() {
        let q = find_to_sql("c", &json!({}), None, Some(&json!({"age": -1})), Some(5), Some(10));
        assert_eq!(
            q.sql,
            "SELECT _jsonb FROM \"c\" ORDER BY _jsonb -> 'age' DESC LIMIT 10 OFFSET 5"
        );
    }

    #[test]
    fn find_inclusion_projection_keeps_id() {
        let q = find_to_sql("c", &json!({}), Some(&json!({"name": 1})), None, None, None);
        assert_eq!(
            q.sql,
            "SELECT jsonb_build_object('_id', _jsonb -> '_id', 'name', _jsonb -> 'name') AS _jsonb FROM \"c\""
        );
    }

    #[test]
    fn find_inclusion_projection_drops_id_when_zero() {
        let q = find_to_sql("c", &json!({}), Some(&json!({"_id": 0, "name": 1})), None, None, None);
        assert_eq!(
            q.sql,
            "SELECT jsonb_build_object('name', _jsonb -> 'name') AS _jsonb FROM \"c\""
        );
    }

    #[test]
    fn quoting_escapes_identifier() {
        let q = find_to_sql("we\"ird", &json!({}), None, None, None, None);
        assert_eq!(q.sql, "SELECT _jsonb FROM \"we\"\"ird\"");
    }

    // ── write ops ────────────────────────────────────────────────────────

    #[test]
    fn insert_single_doc() {
        let q = insert_to_sql("c", &json!({"_id": "1", "n": 5}));
        assert_eq!(q.sql, "INSERT INTO \"c\" (_jsonb) VALUES ($1::jsonb)");
        assert_eq!(q.params, vec!["{\"_id\":\"1\",\"n\":5}"]);
    }

    #[test]
    fn update_set_merges_jsonb() {
        let q = update_to_sql("c", &json!({"_id": "1"}), &json!({"$set": {"a": 1}}));
        assert_eq!(
            q.sql,
            "UPDATE \"c\" SET _jsonb = (_jsonb) || $1::jsonb WHERE _jsonb -> '_id' = $2::jsonb"
        );
        assert_eq!(q.params, vec!["{\"a\":1}", "\"1\""]);
    }

    #[test]
    fn update_unset_drops_keys() {
        let q = update_to_sql("c", &json!({}), &json!({"$unset": {"old": ""}}));
        assert_eq!(q.sql, "UPDATE \"c\" SET _jsonb = _jsonb - 'old'");
        assert!(q.params.is_empty());
    }

    #[test]
    fn update_inc_uses_jsonb_set() {
        let q = update_to_sql("c", &json!({}), &json!({"$inc": {"n": 5}}));
        assert_eq!(
            q.sql,
            "UPDATE \"c\" SET _jsonb = jsonb_set(_jsonb, '{n}', to_jsonb(COALESCE((_jsonb ->> 'n')::numeric, 0) + $1::numeric))"
        );
        assert_eq!(q.params, vec!["5"]);
    }

    #[test]
    fn update_replacement_whole_doc() {
        let q = update_to_sql("c", &json!({"_id": "1"}), &json!({"name": "bob"}));
        assert_eq!(
            q.sql,
            "UPDATE \"c\" SET _jsonb = $1::jsonb WHERE _jsonb -> '_id' = $2::jsonb"
        );
        assert_eq!(q.params, vec!["{\"name\":\"bob\"}", "\"1\""]);
    }

    #[test]
    fn delete_with_filter() {
        let q = delete_to_sql("c", &json!({"_id": "1"}));
        assert_eq!(q.sql, "DELETE FROM \"c\" WHERE _jsonb -> '_id' = $1::jsonb");
        assert_eq!(q.params, vec!["\"1\""]);
    }

    // ── aggregation pipeline → SQL ───────────────────────────────────────

    fn agg(table: &str, pipeline: serde_json::Value) -> SqlQuery {
        let arr = pipeline.as_array().unwrap().clone();
        pipeline_to_sql(table, &arr).expect("pipeline should translate")
    }

    #[test]
    fn agg_group_sum() {
        let q = agg("emp", json!([{"$group": {"_id": "$dept", "total": {"$sum": "$salary"}}}]));
        assert_eq!(
            q.sql,
            "SELECT jsonb_build_object('_id', _jsonb -> 'dept', 'total', to_jsonb(sum((_jsonb ->> 'salary')::numeric))) AS _jsonb FROM \"emp\" GROUP BY _jsonb -> 'dept'"
        );
    }

    #[test]
    fn agg_group_count_null_id() {
        let q = agg("emp", json!([{"$group": {"_id": null, "n": {"$sum": 1}}}]));
        assert_eq!(
            q.sql,
            "SELECT jsonb_build_object('_id', 'null'::jsonb, 'n', to_jsonb(count(*))) AS _jsonb FROM \"emp\""
        );
    }

    #[test]
    fn agg_match_then_group_avg() {
        let q = agg(
            "emp",
            json!([{"$match": {"active": true}}, {"$group": {"_id": "$dept", "avg": {"$avg": "$age"}}}]),
        );
        assert_eq!(
            q.sql,
            "SELECT jsonb_build_object('_id', _jsonb -> 'dept', 'avg', to_jsonb(avg((_jsonb ->> 'age')::numeric))) AS _jsonb FROM \"emp\" WHERE _jsonb -> 'active' = $1::jsonb GROUP BY _jsonb -> 'dept'"
        );
        assert_eq!(q.params, vec!["true"]);
    }

    #[test]
    fn agg_match_sort_limit_linear() {
        let q = agg(
            "c",
            json!([{"$match": {"x": {"$gt": 1}}}, {"$sort": {"x": -1}}, {"$limit": 5}]),
        );
        assert_eq!(
            q.sql,
            "SELECT _jsonb FROM \"c\" WHERE _jsonb -> 'x' > $1::jsonb ORDER BY _jsonb -> 'x' DESC LIMIT 5"
        );
    }

    #[test]
    fn agg_count_stage() {
        let q = agg("c", json!([{"$count": "total"}]));
        assert_eq!(
            q.sql,
            "SELECT jsonb_build_object('total', count(*)) AS _jsonb FROM \"c\""
        );
    }

    #[test]
    fn agg_project_inclusion() {
        let q = agg("c", json!([{"$project": {"name": 1}}]));
        assert_eq!(
            q.sql,
            "SELECT jsonb_build_object('_id', _jsonb -> '_id', 'name', _jsonb -> 'name') AS _jsonb FROM \"c\""
        );
    }

    #[test]
    fn agg_empty_pipeline() {
        let q = agg("c", json!([]));
        assert_eq!(q.sql, "SELECT _jsonb FROM \"c\"");
    }

    #[test]
    fn agg_unsupported_stage_returns_none() {
        let arr = vec![json!({"$unwind": "$tags"})];
        assert!(pipeline_to_sql("c", &arr).is_none());
    }

    #[test]
    fn agg_group_min_max() {
        let q = agg(
            "emp",
            json!([{"$group": {"_id": "$d", "hi": {"$max": "$v"}, "lo": {"$min": "$v"}}}]),
        );
        assert_eq!(
            q.sql,
            "SELECT jsonb_build_object('_id', _jsonb -> 'd', 'hi', to_jsonb(max((_jsonb ->> 'v')::numeric)), 'lo', to_jsonb(min((_jsonb ->> 'v')::numeric))) AS _jsonb FROM \"emp\" GROUP BY _jsonb -> 'd'"
        );
    }
}

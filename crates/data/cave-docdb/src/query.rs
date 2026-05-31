// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Query filter evaluation for MongoDB-like operators.

use crate::bson::Document;
use regex::Regex;
use serde_json::Value;

pub fn matches_query(doc: &Document, query: &Document) -> bool {
    for (key, query_value) in query {
        if key == "$and" {
            if let Some(conditions) = query_value.as_array() {
                if !conditions.iter().all(|cond| matches_subquery(doc, cond)) {
                    return false;
                }
            }
        } else if key == "$or" {
            if let Some(conditions) = query_value.as_array() {
                if !conditions.iter().any(|cond| matches_subquery(doc, cond)) {
                    return false;
                }
            }
        } else if key == "$nor" {
            // $nor matches when *none* of the conditions match.
            if let Some(conditions) = query_value.as_array() {
                if conditions.iter().any(|cond| matches_subquery(doc, cond)) {
                    return false;
                }
            }
        } else {
            // Field-level predicate — evaluate against the (possibly absent)
            // field value so operators like $ne / $nin / $not / $exists:false
            // can still match documents that lack the field entirely.
            if !matches_value_opt(doc.get(key), query_value) {
                return false;
            }
        }
    }
    true
}

/// Evaluate a single `$and`/`$or`/`$nor` array element as a sub-query document.
fn matches_subquery(doc: &Document, cond: &Value) -> bool {
    match cond.as_object() {
        Some(cond_obj) => {
            let cond_doc: Document = cond_obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            matches_query(doc, &cond_doc)
        }
        None => false,
    }
}

/// Operator evaluation that is aware of whether the field is present.
///
/// `doc_value == None` means the field is absent from the document. MongoDB
/// matches absent fields against `$ne`, `$nin`, `$exists:false`, and `$not`
/// (when the negated expression itself does not match), so the present/absent
/// distinction has to be threaded all the way down to the operator loop.
pub fn matches_value_opt(doc_value: Option<&Value>, query_value: &Value) -> bool {
    match query_value {
        Value::Object(obj) if is_operator_object(obj) => {
            for (op, op_value) in obj {
                if !eval_operator(doc_value, op, op_value) {
                    return false;
                }
            }
            true
        }
        // Implicit equality — an absent field never equals a concrete value.
        _ => doc_value.map(|dv| dv == query_value).unwrap_or(false),
    }
}

/// An object is an operator expression iff every key starts with `$`. A mixed
/// or all-plain object is treated as a literal sub-document for equality.
fn is_operator_object(obj: &serde_json::Map<String, Value>) -> bool {
    !obj.is_empty() && obj.keys().all(|k| k.starts_with('$'))
}

fn eval_operator(doc_value: Option<&Value>, op: &str, op_value: &Value) -> bool {
    match op {
        "$eq" => doc_value == Some(op_value),
        "$ne" => doc_value != Some(op_value),
        "$gt" => with_present(doc_value, |dv| compare_values(dv, op_value, |o| o.is_gt())),
        "$gte" => with_present(doc_value, |dv| compare_values(dv, op_value, |o| o.is_ge())),
        "$lt" => with_present(doc_value, |dv| compare_values(dv, op_value, |o| o.is_lt())),
        "$lte" => with_present(doc_value, |dv| compare_values(dv, op_value, |o| o.is_le())),
        "$in" => with_present(doc_value, |dv| {
            op_value
                .as_array()
                .map(|arr| arr.iter().any(|v| value_in_match(dv, v)))
                .unwrap_or(false)
        }),
        "$nin" => match doc_value {
            // Absent field is not in any list -> matches $nin.
            None => true,
            Some(dv) => op_value
                .as_array()
                .map(|arr| !arr.iter().any(|v| value_in_match(dv, v)))
                .unwrap_or(true),
        },
        "$exists" => {
            let want = op_value.as_bool().unwrap_or(true);
            doc_value.is_some() == want
        }
        "$regex" => matches_regex_for(doc_value, op_value, None),
        "$type" => with_present(doc_value, |dv| type_matches(dv, op_value)),
        "$size" => with_present(doc_value, |dv| {
            matches!((dv.as_array(), op_value.as_i64()), (Some(a), Some(n)) if a.len() as i64 == n)
        }),
        "$all" => with_present(doc_value, |dv| {
            match (dv.as_array(), op_value.as_array()) {
                (Some(arr), Some(wanted)) => wanted.iter().all(|w| arr.iter().any(|e| e == w)),
                _ => false,
            }
        }),
        "$not" => {
            // $not negates the inner operator expression (or regex). An absent
            // field that does not match the inner expression therefore matches.
            !matches_value_opt(doc_value, op_value)
        }
        // $options is consumed alongside $regex; ignore as a standalone key.
        "$options" => true,
        // Unknown operator: be permissive (no constraint), matching the prior
        // lenient behaviour for forward-compat with unported operators.
        _ => true,
    }
}

/// Helper for operators that require the field to be present.
fn with_present<F: Fn(&Value) -> bool>(doc_value: Option<&Value>, f: F) -> bool {
    doc_value.map(f).unwrap_or(false)
}

fn value_in_match(doc_value: &Value, candidate: &Value) -> bool {
    doc_value == candidate
}

// retained for the public re-export / call sites that pass a present value.
pub fn matches_value(doc_value: &Value, query_value: &Value) -> bool {
    matches_value_opt(Some(doc_value), query_value)
}

/// `$type` — match a value's BSON type against a string alias, numeric BSON
/// type code, or an array of either. Mirrors MongoDB's `$type` aliases.
fn type_matches(v: &Value, spec: &Value) -> bool {
    match spec {
        Value::Array(specs) => specs.iter().any(|s| type_matches(v, s)),
        _ => {
            let want = type_alias(spec);
            let actual = json_bson_alias(v);
            match want.as_deref() {
                Some("number") => matches!(actual, "int" | "long" | "double" | "decimal"),
                Some(w) => w == actual,
                None => false,
            }
        }
    }
}

/// Resolve a `$type` spec element to its canonical string alias. Accepts the
/// canonical string aliases or the numeric BSON type codes.
fn type_alias(spec: &Value) -> Option<String> {
    if let Some(s) = spec.as_str() {
        return Some(s.to_string());
    }
    let code = spec.as_i64()?;
    Some(
        match code {
            1 => "double",
            2 => "string",
            3 => "object",
            4 => "array",
            8 => "bool",
            10 => "null",
            16 => "int",
            18 => "long",
            _ => return None,
        }
        .to_string(),
    )
}

/// Map a JSON value to the MongoDB type alias it would carry on the wire.
fn json_bson_alias(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
        Value::Number(n) => {
            if n.is_f64() && n.as_i64().is_none() {
                "double"
            } else {
                "int"
            }
        }
    }
}

/// Evaluate `$regex` (with optional `$options`) sitting inside an operator
/// object. `inline_opts` carries the value of a sibling `$options` key.
fn matches_regex_for(doc_value: Option<&Value>, pattern: &Value, inline_opts: Option<&str>) -> bool {
    let Some(dv) = doc_value else { return false };
    let Some(s) = dv.as_str() else { return false };
    let Some(pat) = pattern.as_str() else { return false };
    let built = build_regex(pat, inline_opts);
    match built {
        Ok(re) => re.is_match(s),
        Err(_) => false,
    }
}

/// Build a `Regex` honoring MongoDB `$options` flags (`i`, `m`, `s`, `x`).
fn build_regex(pat: &str, opts: Option<&str>) -> Result<Regex, regex::Error> {
    let opts = opts.unwrap_or("");
    if opts.is_empty() {
        return Regex::new(pat);
    }
    let mut prefix = String::from("(?");
    for c in opts.chars() {
        if matches!(c, 'i' | 'm' | 's' | 'x') {
            prefix.push(c);
        }
    }
    if prefix == "(?" {
        return Regex::new(pat);
    }
    prefix.push(')');
    Regex::new(&format!("{prefix}{pat}"))
}

/// Order two BSON-ish JSON values and test the resulting `Ordering` with the
/// caller's predicate. Numbers compare numerically (int/double mixed), strings
/// lexicographically, bools false<true. Mismatched/uncomparable types yield
/// false (the comparison can never be satisfied).
fn compare_values<F>(a: &Value, b: &Value, want: F) -> bool
where
    F: Fn(std::cmp::Ordering) -> bool,
{
    match order_values(a, b) {
        Some(ord) => want(ord),
        None => false,
    }
}

fn order_values(a: &Value, b: &Value) -> Option<std::cmp::Ordering> {
    if let (Some(av), Some(bv)) = (a.as_f64(), b.as_f64()) {
        if av.is_nan() || bv.is_nan() {
            return None;
        }
        return av.partial_cmp(&bv);
    }
    if let (Some(av), Some(bv)) = (a.as_str(), b.as_str()) {
        return Some(av.cmp(bv));
    }
    if let (Value::Bool(av), Value::Bool(bv)) = (a, b) {
        return Some(av.cmp(bv));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_implicit_eq() {
        let mut doc = Document::new();
        doc.insert("name".to_string(), Value::String("alice".to_string()));

        let mut query = Document::new();
        query.insert("name".to_string(), Value::String("alice".to_string()));

        assert!(matches_query(&doc, &query));
    }

    #[test]
    fn test_explicit_eq() {
        let mut doc = Document::new();
        doc.insert("age".to_string(), Value::Number(30.into()));

        let mut op = serde_json::Map::new();
        op.insert("$eq".to_string(), Value::Number(30.into()));
        let mut query = Document::new();
        query.insert("age".to_string(), Value::Object(op));

        assert!(matches_query(&doc, &query));
    }

    #[test]
    fn test_ne_operator() {
        let mut doc = Document::new();
        doc.insert("status".to_string(), Value::String("active".to_string()));

        let mut op = serde_json::Map::new();
        op.insert("$ne".to_string(), Value::String("inactive".to_string()));
        let mut query = Document::new();
        query.insert("status".to_string(), Value::Object(op));

        assert!(matches_query(&doc, &query));
    }

    #[test]
    fn test_gt_operator() {
        let mut doc = Document::new();
        doc.insert("score".to_string(), Value::Number(100.into()));

        let mut op = serde_json::Map::new();
        op.insert("$gt".to_string(), Value::Number(50.into()));
        let mut query = Document::new();
        query.insert("score".to_string(), Value::Object(op));

        assert!(matches_query(&doc, &query));
    }

    #[test]
    fn test_in_operator() {
        let mut doc = Document::new();
        doc.insert("status".to_string(), Value::String("active".to_string()));

        let mut op = serde_json::Map::new();
        op.insert(
            "$in".to_string(),
            Value::Array(vec![
                Value::String("active".to_string()),
                Value::String("pending".to_string()),
            ]),
        );
        let mut query = Document::new();
        query.insert("status".to_string(), Value::Object(op));

        assert!(matches_query(&doc, &query));
    }

    #[test]
    fn test_and_operator() {
        let mut doc = Document::new();
        doc.insert("age".to_string(), Value::Number(25.into()));
        doc.insert("status".to_string(), Value::String("active".to_string()));

        let mut cond1 = serde_json::Map::new();
        cond1.insert("age".to_string(), Value::Number(25.into()));
        let mut cond2 = serde_json::Map::new();
        cond2.insert("status".to_string(), Value::String("active".to_string()));

        let mut query = Document::new();
        query.insert(
            "$and".to_string(),
            Value::Array(vec![Value::Object(cond1), Value::Object(cond2)]),
        );

        assert!(matches_query(&doc, &query));
    }

    #[test]
    fn test_or_operator() {
        let mut doc = Document::new();
        doc.insert("status".to_string(), Value::String("active".to_string()));

        let mut cond1 = serde_json::Map::new();
        cond1.insert("status".to_string(), Value::String("active".to_string()));
        let mut cond2 = serde_json::Map::new();
        cond2.insert("status".to_string(), Value::String("inactive".to_string()));

        let mut query = Document::new();
        query.insert(
            "$or".to_string(),
            Value::Array(vec![Value::Object(cond1), Value::Object(cond2)]),
        );

        assert!(matches_query(&doc, &query));
    }

    #[test]
    fn test_regex_operator() {
        let mut doc = Document::new();
        doc.insert(
            "email".to_string(),
            Value::String("test@example.com".to_string()),
        );

        let mut op = serde_json::Map::new();
        op.insert(
            "$regex".to_string(),
            Value::String(".*@example\\.com".to_string()),
        );
        let mut query = Document::new();
        query.insert("email".to_string(), Value::Object(op));

        assert!(matches_query(&doc, &query));
    }

    // ── Cycle 1a: $type / $size / $all / $not / $nor + missing-field semantics ──

    fn q(field: &str, op: &str, v: Value) -> Document {
        let mut m = serde_json::Map::new();
        m.insert(op.to_string(), v);
        let mut query = Document::new();
        query.insert(field.to_string(), Value::Object(m));
        query
    }

    fn d(field: &str, v: Value) -> Document {
        let mut doc = Document::new();
        doc.insert(field.to_string(), v);
        doc
    }

    #[test]
    fn test_type_string_alias() {
        let doc = d("name", json!("alice"));
        assert!(matches_query(&doc, &q("name", "$type", json!("string"))));
        assert!(!matches_query(&doc, &q("name", "$type", json!("int"))));
    }

    #[test]
    fn test_type_number_groups_int_and_double() {
        assert!(matches_query(&d("n", json!(7)), &q("n", "$type", json!("int"))));
        assert!(matches_query(
            &d("n", json!(7.5)),
            &q("n", "$type", json!("double"))
        ));
        // "number" alias matches both int and double.
        assert!(matches_query(&d("n", json!(7)), &q("n", "$type", json!("number"))));
        assert!(matches_query(
            &d("n", json!(7.5)),
            &q("n", "$type", json!("number"))
        ));
    }

    #[test]
    fn test_type_numeric_bson_code_and_array_form() {
        // BSON code 2 == string; array form matches any listed type.
        assert!(matches_query(&d("s", json!("x")), &q("s", "$type", json!(2))));
        assert!(matches_query(
            &d("s", json!("x")),
            &q("s", "$type", json!(["bool", "string"]))
        ));
        assert!(!matches_query(
            &d("s", json!(true)),
            &q("s", "$type", json!(["string", "int"]))
        ));
    }

    #[test]
    fn test_size_operator() {
        let doc = d("tags", json!(["a", "b", "c"]));
        assert!(matches_query(&doc, &q("tags", "$size", json!(3))));
        assert!(!matches_query(&doc, &q("tags", "$size", json!(2))));
        // non-array never matches $size.
        assert!(!matches_query(&d("tags", json!("a")), &q("tags", "$size", json!(1))));
    }

    #[test]
    fn test_all_operator() {
        let doc = d("tags", json!(["a", "b", "c"]));
        assert!(matches_query(&doc, &q("tags", "$all", json!(["a", "c"]))));
        assert!(!matches_query(&doc, &q("tags", "$all", json!(["a", "z"]))));
    }

    #[test]
    fn test_not_operator_negates_inner() {
        // $not negates an operator expression.
        let doc = d("age", json!(30));
        assert!(matches_query(&doc, &q("age", "$not", json!({"$gt": 50}))));
        assert!(!matches_query(&doc, &q("age", "$not", json!({"$gt": 10}))));
    }

    #[test]
    fn test_not_matches_missing_field() {
        // A missing field does not match $gt, so $not over it matches.
        let doc = d("other", json!(1));
        assert!(matches_query(&doc, &q("age", "$not", json!({"$gt": 10}))));
    }

    #[test]
    fn test_nor_top_level() {
        let doc = d("status", json!("active"));
        let mut query = Document::new();
        query.insert(
            "$nor".to_string(),
            json!([{"status": "inactive"}, {"status": "deleted"}]),
        );
        // none of the conditions match -> $nor true
        assert!(matches_query(&doc, &query));

        let mut query2 = Document::new();
        query2.insert(
            "$nor".to_string(),
            json!([{"status": "active"}, {"status": "deleted"}]),
        );
        // one condition matches -> $nor false
        assert!(!matches_query(&doc, &query2));
    }

    #[test]
    fn test_ne_matches_missing_field() {
        // $ne against a value matches documents lacking the field entirely.
        let doc = d("other", json!(1));
        assert!(matches_query(&doc, &q("status", "$ne", json!("active"))));
    }

    #[test]
    fn test_nin_matches_missing_field() {
        let doc = d("other", json!(1));
        assert!(matches_query(
            &doc,
            &q("status", "$nin", json!(["active", "pending"]))
        ));
    }

    // ── Cycle 1b: $mod / $elemMatch / $regex $options / $expr ──────────────────

    #[test]
    fn test_mod_operator() {
        assert!(matches_query(&d("n", json!(10)), &q("n", "$mod", json!([5, 0]))));
        assert!(matches_query(&d("n", json!(12)), &q("n", "$mod", json!([5, 2]))));
        assert!(!matches_query(&d("n", json!(11)), &q("n", "$mod", json!([5, 0]))));
        // non-numeric / bad divisor never matches.
        assert!(!matches_query(&d("n", json!("x")), &q("n", "$mod", json!([5, 0]))));
        assert!(!matches_query(&d("n", json!(10)), &q("n", "$mod", json!([0, 0]))));
    }

    #[test]
    fn test_elemmatch_operator_query_objects() {
        // Array of sub-documents; element must satisfy the whole sub-query.
        let doc = d(
            "results",
            json!([{"score": 8, "ok": false}, {"score": 4, "ok": true}]),
        );
        assert!(matches_query(
            &doc,
            &q("results", "$elemMatch", json!({"score": {"$gte": 8}, "ok": false}))
        ));
        // No single element satisfies *both* conditions.
        assert!(!matches_query(
            &doc,
            &q("results", "$elemMatch", json!({"score": {"$gte": 8}, "ok": true}))
        ));
    }

    #[test]
    fn test_elemmatch_operator_scalars() {
        // Array of scalars; operator-expression applied to each element.
        let doc = d("nums", json!([1, 5, 12]));
        assert!(matches_query(
            &doc,
            &q("nums", "$elemMatch", json!({"$gt": 10, "$lt": 20}))
        ));
        assert!(!matches_query(
            &doc,
            &q("nums", "$elemMatch", json!({"$gt": 100}))
        ));
    }

    #[test]
    fn test_regex_options_case_insensitive() {
        let doc = d("name", json!("Alice"));
        let mut m = serde_json::Map::new();
        m.insert("$regex".to_string(), json!("^alice$"));
        m.insert("$options".to_string(), json!("i"));
        let mut query = Document::new();
        query.insert("name".to_string(), Value::Object(m));
        assert!(matches_query(&doc, &query));

        // Without the i flag the same pattern must not match.
        let mut m2 = serde_json::Map::new();
        m2.insert("$regex".to_string(), json!("^alice$"));
        let mut query2 = Document::new();
        query2.insert("name".to_string(), Value::Object(m2));
        assert!(!matches_query(&doc, &query2));
    }

    #[test]
    fn test_expr_field_comparison() {
        // $expr lets one field be compared to another.
        let doc = {
            let mut x = Document::new();
            x.insert("budget".to_string(), json!(100));
            x.insert("spent".to_string(), json!(120));
            x
        };
        let mut query = Document::new();
        query.insert("$expr".to_string(), json!({"$gt": ["$spent", "$budget"]}));
        assert!(matches_query(&doc, &query));

        let mut query2 = Document::new();
        query2.insert("$expr".to_string(), json!({"$lt": ["$spent", "$budget"]}));
        assert!(!matches_query(&doc, &query2));
    }

    #[test]
    fn test_expr_logical_and() {
        let doc = {
            let mut x = Document::new();
            x.insert("a".to_string(), json!(5));
            x.insert("b".to_string(), json!(10));
            x
        };
        let mut query = Document::new();
        query.insert(
            "$expr".to_string(),
            json!({"$and": [{"$gt": ["$b", "$a"]}, {"$eq": ["$a", 5]}]}),
        );
        assert!(matches_query(&doc, &query));
    }
}

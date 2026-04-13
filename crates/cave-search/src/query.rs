//! Query DSL execution layer.
//!
//! Parses and executes the OpenSearch / Elasticsearch query DSL expressed as
//! `serde_json::Value`.  Each query type is handled by a dedicated function;
//! complex queries (bool) are evaluated recursively.
//!
//! Score combination rules mirror OpenSearch behaviour:
//! - `must`   clauses: scores are summed, all must match.
//! - `should` clauses: scores are summed; at least one must match unless
//!                     `minimum_should_match` is 0.
//! - `filter` clauses: must match, score is not affected.
//! - `must_not` clauses: matched documents are excluded.

use serde_json::Value;
use std::collections::{HashMap, HashSet};

use crate::engine::IndexData;
use crate::index::analyze_text;

// ─────────────────────────────────────────────────────────────────────────────
// Main dispatch
// ─────────────────────────────────────────────────────────────────────────────

/// Execute a query value against `idx`.  Returns `(doc_id, score)` pairs in
/// descending score order.
pub fn execute_query(q: &Value, idx: &IndexData) -> Vec<(String, f64)> {
    if q.is_null() {
        return idx.match_all();
    }

    // match_all / match_none
    if q.get("match_all").is_some() {
        return idx.match_all();
    }
    if q.get("match_none").is_some() {
        return vec![];
    }

    if let Some(bool_q) = q.get("bool") {
        return exec_bool(bool_q, idx);
    }
    if let Some(match_q) = q.get("match") {
        return exec_match(match_q, idx);
    }
    if let Some(mm_q) = q.get("multi_match") {
        return exec_multi_match(mm_q, idx);
    }
    if let Some(term_q) = q.get("term") {
        return exec_term(term_q, idx);
    }
    if let Some(terms_q) = q.get("terms") {
        return exec_terms(terms_q, idx);
    }
    if let Some(range_q) = q.get("range") {
        return exec_range(range_q, idx);
    }
    if let Some(prefix_q) = q.get("prefix") {
        return exec_prefix(prefix_q, idx);
    }
    if let Some(fuzzy_q) = q.get("fuzzy") {
        return exec_fuzzy(fuzzy_q, idx);
    }
    if let Some(wildcard_q) = q.get("wildcard") {
        return exec_wildcard(wildcard_q, idx);
    }
    if let Some(exists_q) = q.get("exists") {
        return exec_exists(exists_q, idx);
    }
    if let Some(ids_q) = q.get("ids") {
        return exec_ids(ids_q, idx);
    }
    if let Some(qs_q) = q.get("query_string") {
        return exec_query_string(qs_q, idx);
    }
    if let Some(simple_q) = q.get("simple_query_string") {
        return exec_simple_query_string(simple_q, idx);
    }
    if let Some(fn_score) = q.get("function_score") {
        return exec_function_score(fn_score, idx);
    }
    if let Some(dis_max) = q.get("dis_max") {
        return exec_dis_max(dis_max, idx);
    }
    if let Some(nested) = q.get("nested") {
        return exec_nested(nested, idx);
    }
    if let Some(constant) = q.get("constant_score") {
        return exec_constant_score(constant, idx);
    }

    // Fallback — unknown query type → empty.
    vec![]
}

// ─────────────────────────────────────────────────────────────────────────────
// bool query
// ─────────────────────────────────────────────────────────────────────────────

fn exec_bool(bool_q: &Value, idx: &IndexData) -> Vec<(String, f64)> {
    let must = bool_q.get("must");
    let should = bool_q.get("should");
    let must_not = bool_q.get("must_not");
    let filter = bool_q.get("filter");
    let minimum_should_match = bool_q
        .get("minimum_should_match")
        .and_then(|v| v.as_u64())
        .unwrap_or(1) as usize;

    // ── must ────────────────────────────────────────────────────────────────
    let mut must_sets: Vec<HashMap<String, f64>> = Vec::new();
    if let Some(must_clauses) = must {
        for clause in normalise_clauses(must_clauses) {
            let results = execute_query(clause, idx);
            let map: HashMap<String, f64> = results.into_iter().collect();
            must_sets.push(map);
        }
    }

    // ── filter ──────────────────────────────────────────────────────────────
    let mut filter_sets: Vec<HashSet<String>> = Vec::new();
    if let Some(filter_clauses) = filter {
        for clause in normalise_clauses(filter_clauses) {
            let results = execute_query(clause, idx);
            let set: HashSet<String> = results.into_iter().map(|(id, _)| id).collect();
            filter_sets.push(set);
        }
    }

    // ── must_not ────────────────────────────────────────────────────────────
    let mut excluded: HashSet<String> = HashSet::new();
    if let Some(must_not_clauses) = must_not {
        for clause in normalise_clauses(must_not_clauses) {
            for (id, _) in execute_query(clause, idx) {
                excluded.insert(id);
            }
        }
    }

    // ── should ──────────────────────────────────────────────────────────────
    let mut should_scores: HashMap<String, (f64, usize)> = HashMap::new(); // (score, match_count)
    let should_clause_count = should
        .and_then(|s| s.as_array())
        .map(|a| a.len())
        .unwrap_or(0);

    if let Some(should_clauses) = should {
        for clause in normalise_clauses(should_clauses) {
            for (id, score) in execute_query(clause, idx) {
                let e = should_scores.entry(id).or_insert((0.0, 0));
                e.0 += score;
                e.1 += 1;
            }
        }
    }

    // ── Combine ──────────────────────────────────────────────────────────────
    // Start from the universe of documents that satisfy ALL must clauses.
    let candidate_ids: HashSet<String> = if must_sets.is_empty() {
        // No must clauses → all docs are candidates (further filtered below).
        idx.documents.keys().cloned().collect()
    } else {
        // Intersection of all must-clause match sets.
        let first: HashSet<String> = must_sets[0].keys().cloned().collect();
        must_sets[1..].iter().fold(first, |acc, set| {
            acc.into_iter().filter(|id| set.contains_key(id)).collect()
        })
    };

    let mut result_scores: HashMap<String, f64> = HashMap::new();

    for id in candidate_ids {
        if excluded.contains(&id) { continue; }

        // Check filter clauses.
        if !filter_sets.iter().all(|s| s.contains(&id)) { continue; }

        // Check minimum_should_match.
        if should_clause_count > 0 {
            let (should_score, should_count) =
                should_scores.get(&id).copied().unwrap_or((0.0, 0));

            let effective_min = if must_sets.is_empty() {
                // Pure should query.
                minimum_should_match.max(1)
            } else {
                // Mixed must+should: should is optional by default.
                0
            };

            if should_count < effective_min { continue; }
            *result_scores.entry(id.clone()).or_default() += should_score;
        }

        // Add must scores.
        let must_score: f64 = must_sets.iter().map(|m| m.get(&id).copied().unwrap_or(0.0)).sum();
        *result_scores.entry(id.clone()).or_default() += must_score;
    }

    let mut result: Vec<(String, f64)> = result_scores.into_iter().collect();
    result.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    result
}

/// Accept either an array `[{...}, ...]` or a single object `{...}`.
fn normalise_clauses(v: &Value) -> Vec<&Value> {
    match v {
        Value::Array(arr) => arr.iter().collect(),
        obj => vec![obj],
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// match query
// ─────────────────────────────────────────────────────────────────────────────

fn exec_match(match_q: &Value, idx: &IndexData) -> Vec<(String, f64)> {
    let Some(obj) = match_q.as_object() else { return vec![] };

    let mut all_results: HashMap<String, f64> = HashMap::new();

    for (field, options) in obj {
        let (query_text, operator_and, boost) = parse_match_options(options);
        let tokens = analyze_text(&query_text, &idx.mapping, field);
        let terms: Vec<String> = tokens.into_iter().map(|t| t.text).collect();

        for (id, score) in idx.score_terms(field, &terms, operator_and) {
            *all_results.entry(id).or_default() += score * boost;
        }
    }

    sorted_desc(all_results)
}

fn parse_match_options(v: &Value) -> (String, bool, f64) {
    match v {
        Value::String(s) => (s.clone(), false, 1.0),
        Value::Object(m) => {
            let query = m.get("query").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let operator_and = m.get("operator").and_then(|v| v.as_str()) == Some("and");
            let boost = m.get("boost").and_then(|v| v.as_f64()).unwrap_or(1.0);
            (query, operator_and, boost)
        }
        _ => (String::new(), false, 1.0),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// multi_match query
// ─────────────────────────────────────────────────────────────────────────────

fn exec_multi_match(mm_q: &Value, idx: &IndexData) -> Vec<(String, f64)> {
    let query_text = mm_q.get("query").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let operator_and = mm_q.get("operator").and_then(|v| v.as_str()) == Some("and");
    let mm_type = mm_q.get("type").and_then(|v| v.as_str()).unwrap_or("best_fields");
    let tie_breaker = mm_q.get("tie_breaker").and_then(|v| v.as_f64()).unwrap_or(0.0);

    let fields: Vec<(String, f64)> = match mm_q.get("fields") {
        Some(Value::Array(arr)) => arr
            .iter()
            .filter_map(|v| v.as_str())
            .map(|s| {
                if let Some((f, b)) = s.split_once('^') {
                    (f.to_string(), b.parse::<f64>().unwrap_or(1.0))
                } else {
                    (s.to_string(), 1.0)
                }
            })
            .collect(),
        _ => idx.mapping.properties.keys().map(|k| (k.clone(), 1.0)).collect(),
    };

    let mut doc_field_scores: HashMap<String, Vec<f64>> = HashMap::new();

    for (field, boost) in &fields {
        let tokens = analyze_text(&query_text, &idx.mapping, field);
        let terms: Vec<String> = tokens.into_iter().map(|t| t.text).collect();
        for (id, score) in idx.score_terms(field, &terms, operator_and) {
            doc_field_scores.entry(id).or_default().push(score * boost);
        }
    }

    let combined: HashMap<String, f64> = doc_field_scores
        .into_iter()
        .map(|(id, mut scores)| {
            scores.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
            let combined_score = match mm_type {
                "best_fields" => {
                    let best = scores[0];
                    let rest: f64 = scores[1..].iter().sum::<f64>() * tie_breaker;
                    best + rest
                }
                "most_fields" => scores.iter().sum(),
                "cross_fields" => scores.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
                _ => scores.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
            };
            (id, combined_score)
        })
        .collect();

    sorted_desc(combined)
}

// ─────────────────────────────────────────────────────────────────────────────
// term / terms queries
// ─────────────────────────────────────────────────────────────────────────────

fn exec_term(term_q: &Value, idx: &IndexData) -> Vec<(String, f64)> {
    let Some(obj) = term_q.as_object() else { return vec![] };
    let mut results: HashMap<String, f64> = HashMap::new();

    for (field, options) in obj {
        let (value, boost) = parse_term_options(options);
        for (id, score) in idx.exact_term(field, &value.to_lowercase()) {
            *results.entry(id).or_default() += score * boost;
        }
    }

    sorted_desc(results)
}

fn parse_term_options(v: &Value) -> (String, f64) {
    match v {
        Value::String(s) => (s.clone(), 1.0),
        Value::Number(n) => (n.to_string(), 1.0),
        Value::Bool(b) => (b.to_string(), 1.0),
        Value::Object(m) => {
            let value = m.get("value")
                .map(|v| match v {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                })
                .unwrap_or_default();
            let boost = m.get("boost").and_then(|v| v.as_f64()).unwrap_or(1.0);
            (value, boost)
        }
        _ => (String::new(), 1.0),
    }
}

fn exec_terms(terms_q: &Value, idx: &IndexData) -> Vec<(String, f64)> {
    let Some(obj) = terms_q.as_object() else { return vec![] };
    let mut results: HashMap<String, f64> = HashMap::new();

    for (field, values) in obj {
        if field == "boost" { continue; }
        let vals: Vec<String> = match values {
            Value::Array(arr) => arr.iter().map(|v| match v {
                Value::String(s) => s.to_lowercase(),
                other => other.to_string().to_lowercase(),
            }).collect(),
            _ => continue,
        };
        for (id, score) in idx.terms_any(field, &vals) {
            *results.entry(id).or_default() += score;
        }
    }

    sorted_desc(results)
}

// ─────────────────────────────────────────────────────────────────────────────
// range query
// ─────────────────────────────────────────────────────────────────────────────

fn exec_range(range_q: &Value, idx: &IndexData) -> Vec<(String, f64)> {
    let Some(obj) = range_q.as_object() else { return vec![] };
    let mut results: HashMap<String, f64> = HashMap::new();

    for (field, opts) in obj {
        let gte = opts.get("gte");
        let gt = opts.get("gt");
        let lte = opts.get("lte");
        let lt = opts.get("lt");
        let boost = opts.get("boost").and_then(|v| v.as_f64()).unwrap_or(1.0);

        for (id, score) in idx.range_match(field, gte, gt, lte, lt) {
            *results.entry(id).or_default() += score * boost;
        }
    }

    sorted_desc(results)
}

// ─────────────────────────────────────────────────────────────────────────────
// prefix query
// ─────────────────────────────────────────────────────────────────────────────

fn exec_prefix(prefix_q: &Value, idx: &IndexData) -> Vec<(String, f64)> {
    let Some(obj) = prefix_q.as_object() else { return vec![] };
    let mut results: HashMap<String, f64> = HashMap::new();

    for (field, options) in obj {
        let (value, boost) = match options {
            Value::String(s) => (s.to_lowercase(), 1.0),
            Value::Object(m) => {
                let v = m.get("value").and_then(|v| v.as_str()).unwrap_or("").to_lowercase();
                let b = m.get("boost").and_then(|v| v.as_f64()).unwrap_or(1.0);
                (v, b)
            }
            _ => continue,
        };
        for (id, score) in idx.prefix_match(field, &value) {
            *results.entry(id).or_default() += score * boost;
        }
    }

    sorted_desc(results)
}

// ─────────────────────────────────────────────────────────────────────────────
// fuzzy query
// ─────────────────────────────────────────────────────────────────────────────

fn exec_fuzzy(fuzzy_q: &Value, idx: &IndexData) -> Vec<(String, f64)> {
    let Some(obj) = fuzzy_q.as_object() else { return vec![] };
    let mut results: HashMap<String, f64> = HashMap::new();

    for (field, options) in obj {
        let (value, fuzziness, boost) = match options {
            Value::String(s) => (s.to_lowercase(), 1u32, 1.0),
            Value::Object(m) => {
                let v = m.get("value").and_then(|v| v.as_str()).unwrap_or("").to_lowercase();
                let fuzz = m.get("fuzziness")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(1) as u32;
                let b = m.get("boost").and_then(|v| v.as_f64()).unwrap_or(1.0);
                (v, fuzz, b)
            }
            _ => continue,
        };
        for (id, score) in idx.fuzzy_match(field, &value, fuzziness) {
            *results.entry(id).or_default() += score * boost;
        }
    }

    sorted_desc(results)
}

// ─────────────────────────────────────────────────────────────────────────────
// wildcard query
// ─────────────────────────────────────────────────────────────────────────────

fn exec_wildcard(wildcard_q: &Value, idx: &IndexData) -> Vec<(String, f64)> {
    let Some(obj) = wildcard_q.as_object() else { return vec![] };
    let mut results: HashMap<String, f64> = HashMap::new();

    for (field, options) in obj {
        let (value, boost) = match options {
            Value::String(s) => (s.to_lowercase(), 1.0),
            Value::Object(m) => {
                let v = m.get("value").and_then(|v| v.as_str()).unwrap_or("").to_lowercase();
                let b = m.get("boost").and_then(|v| v.as_f64()).unwrap_or(1.0);
                (v, b)
            }
            _ => continue,
        };
        for (id, score) in idx.wildcard_match(field, &value) {
            *results.entry(id).or_default() += score * boost;
        }
    }

    sorted_desc(results)
}

// ─────────────────────────────────────────────────────────────────────────────
// exists query
// ─────────────────────────────────────────────────────────────────────────────

fn exec_exists(exists_q: &Value, idx: &IndexData) -> Vec<(String, f64)> {
    let field = exists_q.get("field").and_then(|v| v.as_str()).unwrap_or("");
    idx.exists_match(field)
}

// ─────────────────────────────────────────────────────────────────────────────
// ids query
// ─────────────────────────────────────────────────────────────────────────────

fn exec_ids(ids_q: &Value, idx: &IndexData) -> Vec<(String, f64)> {
    let values: Vec<String> = ids_q
        .get("values")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    idx.ids_match(&values)
}

// ─────────────────────────────────────────────────────────────────────────────
// query_string query
// ─────────────────────────────────────────────────────────────────────────────

fn exec_query_string(qs_q: &Value, idx: &IndexData) -> Vec<(String, f64)> {
    let query = qs_q.get("query").and_then(|v| v.as_str()).unwrap_or("");
    let default_field = qs_q
        .get("default_field")
        .and_then(|v| v.as_str())
        .unwrap_or("_all");

    // Simple implementation: treat each space-separated token as a should term.
    let tokens: Vec<String> = query.split_whitespace().map(|s| s.to_lowercase()).collect();

    if default_field == "_all" {
        // Search across all mapped fields.
        let fields: Vec<String> = idx.mapping.properties.keys().cloned().collect();
        let mut scores: HashMap<String, f64> = HashMap::new();
        for field in &fields {
            for (id, s) in idx.score_terms(field, &tokens, false) {
                *scores.entry(id).or_default() += s;
            }
        }
        sorted_desc(scores)
    } else {
        let map: HashMap<String, f64> =
            idx.score_terms(default_field, &tokens, false).into_iter().collect();
        sorted_desc(map)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// simple_query_string query
// ─────────────────────────────────────────────────────────────────────────────

fn exec_simple_query_string(qs_q: &Value, idx: &IndexData) -> Vec<(String, f64)> {
    // Reuse query_string logic for simplicity.
    exec_query_string(qs_q, idx)
}

// ─────────────────────────────────────────────────────────────────────────────
// function_score query
// ─────────────────────────────────────────────────────────────────────────────

fn exec_function_score(fn_score: &Value, idx: &IndexData) -> Vec<(String, f64)> {
    let inner_query = fn_score.get("query");
    let boost = fn_score.get("boost").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let weight = fn_score.get("weight").and_then(|v| v.as_f64()).unwrap_or(1.0);

    let mut results = inner_query
        .map(|q| execute_query(q, idx))
        .unwrap_or_else(|| idx.match_all());

    // Apply scalar boost/weight to all scores.
    for (_, score) in &mut results {
        *score *= boost * weight;
    }

    results
}

// ─────────────────────────────────────────────────────────────────────────────
// dis_max query
// ─────────────────────────────────────────────────────────────────────────────

fn exec_dis_max(dis_max: &Value, idx: &IndexData) -> Vec<(String, f64)> {
    let tie_breaker = dis_max.get("tie_breaker").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let Some(queries) = dis_max.get("queries").and_then(|v| v.as_array()) else {
        return vec![];
    };

    let mut doc_scores: HashMap<String, Vec<f64>> = HashMap::new();
    for q in queries {
        for (id, score) in execute_query(q, idx) {
            doc_scores.entry(id).or_default().push(score);
        }
    }

    let combined: HashMap<String, f64> = doc_scores
        .into_iter()
        .map(|(id, mut scores)| {
            scores.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
            let best = scores[0];
            let rest_sum: f64 = scores[1..].iter().sum::<f64>() * tie_breaker;
            (id, best + rest_sum)
        })
        .collect();

    sorted_desc(combined)
}

// ─────────────────────────────────────────────────────────────────────────────
// nested query (simplified — no actual nesting; delegates to inner query)
// ─────────────────────────────────────────────────────────────────────────────

fn exec_nested(nested: &Value, idx: &IndexData) -> Vec<(String, f64)> {
    let Some(inner) = nested.get("query") else { return vec![] };
    execute_query(inner, idx)
}

// ─────────────────────────────────────────────────────────────────────────────
// constant_score query
// ─────────────────────────────────────────────────────────────────────────────

fn exec_constant_score(constant: &Value, idx: &IndexData) -> Vec<(String, f64)> {
    let boost = constant.get("boost").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let Some(filter) = constant.get("filter") else { return vec![] };
    execute_query(filter, idx)
        .into_iter()
        .map(|(id, _)| (id, boost))
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

fn sorted_desc(map: HashMap<String, f64>) -> Vec<(String, f64)> {
    let mut v: Vec<_> = map.into_iter().collect();
    v.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    v
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::IndexData;
    use crate::models::{Document, FieldMapping, FieldType, IndexMapping, IndexSettings};
    use serde_json::json;

    fn make_idx() -> IndexData {
        let mut mapping = IndexMapping::default();
        for f in &["title", "body", "category", "price"] {
            mapping.properties.insert(f.to_string(), FieldMapping {
                field_type: if *f == "price" { FieldType::Float } else if *f == "category" { FieldType::Keyword } else { FieldType::Text },
                ..Default::default()
            });
        }
        IndexData::new("test", mapping, IndexSettings::default())
    }

    fn doc(id: &str, fields: &[(&str, Value)]) -> Document {
        let source = fields.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
        Document::with_id(id, "test", source)
    }

    #[test]
    fn match_all_returns_all() {
        let mut idx = make_idx();
        idx.add_document(doc("1", &[("title", json!("hello"))]));
        idx.add_document(doc("2", &[("title", json!("world"))]));
        let res = execute_query(&json!({"match_all": {}}), &idx);
        assert_eq!(res.len(), 2);
    }

    #[test]
    fn match_query_returns_matching_docs() {
        let mut idx = make_idx();
        idx.add_document(doc("1", &[("title", json!("rust programming"))]));
        idx.add_document(doc("2", &[("title", json!("python scripting"))]));
        let res = execute_query(&json!({"match": {"title": "rust"}}), &idx);
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].0, "1");
    }

    #[test]
    fn bool_must_requires_all_clauses() {
        let mut idx = make_idx();
        idx.add_document(doc("1", &[("title", json!("rust programming language"))]));
        idx.add_document(doc("2", &[("title", json!("rust oxidation chemistry"))]));
        idx.add_document(doc("3", &[("title", json!("python programming tutorial"))]));

        let q = json!({
            "bool": {
                "must": [
                    {"match": {"title": "rust"}},
                    {"match": {"title": "programming"}}
                ]
            }
        });
        let res = execute_query(&q, &idx);
        let ids: Vec<_> = res.iter().map(|(id, _)| id.as_str()).collect();
        assert!(ids.contains(&"1"));
        assert!(!ids.contains(&"2"));
    }

    #[test]
    fn bool_must_not_excludes_docs() {
        let mut idx = make_idx();
        idx.add_document(doc("1", &[("title", json!("rust programming"))]));
        idx.add_document(doc("2", &[("title", json!("deleted article"))]));

        let q = json!({
            "bool": {
                "must": [{"match_all": {}}],
                "must_not": [{"term": {"title": "deleted"}}]
            }
        });
        let res = execute_query(&q, &idx);
        let ids: Vec<_> = res.iter().map(|(id, _)| id.as_str()).collect();
        assert!(ids.contains(&"1"));
        assert!(!ids.contains(&"2"));
    }

    #[test]
    fn range_query_numeric() {
        let mut idx = make_idx();
        idx.add_document(doc("1", &[("price", json!(10.0))]));
        idx.add_document(doc("2", &[("price", json!(50.0))]));
        idx.add_document(doc("3", &[("price", json!(5.0))]));

        let res = execute_query(&json!({"range": {"price": {"gte": 20.0}}}), &idx);
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].0, "2");
    }

    #[test]
    fn term_query_exact_match() {
        let mut idx = make_idx();
        idx.add_document(doc("1", &[("category", json!("news"))]));
        idx.add_document(doc("2", &[("category", json!("sports"))]));

        let res = execute_query(&json!({"term": {"category": "news"}}), &idx);
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].0, "1");
    }

    #[test]
    fn terms_query_any_value() {
        let mut idx = make_idx();
        idx.add_document(doc("1", &[("category", json!("news"))]));
        idx.add_document(doc("2", &[("category", json!("sports"))]));
        idx.add_document(doc("3", &[("category", json!("tech"))]));

        let res = execute_query(&json!({"terms": {"category": ["news", "sports"]}}), &idx);
        let ids: Vec<_> = res.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&"1"));
        assert!(ids.contains(&"2"));
    }

    #[test]
    fn prefix_query_matches_prefix() {
        let mut idx = make_idx();
        idx.add_document(doc("1", &[("title", json!("testing 123"))]));
        idx.add_document(doc("2", &[("title", json!("hello world"))]));

        let res = execute_query(&json!({"prefix": {"title": "test"}}), &idx);
        assert!(!res.is_empty());
        assert_eq!(res[0].0, "1");
    }

    #[test]
    fn multi_match_searches_multiple_fields() {
        let mut idx = make_idx();
        idx.add_document(doc("1", &[("title", json!("rust")), ("body", json!("fast language"))]));
        idx.add_document(doc("2", &[("title", json!("python")), ("body", json!("dynamic language"))]));

        let q = json!({"multi_match": {"query": "rust language", "fields": ["title", "body"]}});
        let res = execute_query(&q, &idx);
        assert!(!res.is_empty());
    }

    #[test]
    fn exists_query_finds_docs_with_field() {
        let mut idx = make_idx();
        idx.add_document(doc("1", &[("title", json!("with title"))]));
        idx.add_document(doc("2", &[("body", json!("no title here"))]));

        let res = execute_query(&json!({"exists": {"field": "title"}}), &idx);
        let ids: Vec<_> = res.iter().map(|(id, _)| id.as_str()).collect();
        assert!(ids.contains(&"1"));
        assert!(!ids.contains(&"2"));
    }

    #[test]
    fn ids_query_matches_by_id() {
        let mut idx = make_idx();
        idx.add_document(doc("abc", &[("title", json!("doc abc"))]));
        idx.add_document(doc("xyz", &[("title", json!("doc xyz"))]));

        let res = execute_query(&json!({"ids": {"values": ["abc"]}}), &idx);
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].0, "abc");
    }
}

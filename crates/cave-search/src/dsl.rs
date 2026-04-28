//! OpenSearch Query DSL evaluator.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::index::{IndexData, StoredDoc};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Query {
    MatchAll,
    Term { field: String, value: Value },
    Terms { field: String, values: Vec<Value> },
    Match { field: String, value: String },
    Range { field: String, gte: Option<f64>, lte: Option<f64>, gt: Option<f64>, lt: Option<f64> },
    Bool {
        #[serde(default)]
        must: Vec<Query>,
        #[serde(default)]
        should: Vec<Query>,
        #[serde(default)]
        must_not: Vec<Query>,
        #[serde(default)]
        filter: Vec<Query>,
        #[serde(default)]
        minimum_should_match: Option<usize>,
    },
    Exists { field: String },
    Prefix { field: String, value: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchHit {
    pub index: String,
    pub id: String,
    pub score: f64,
    pub source: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchResponse {
    pub took: u64,
    pub timed_out: bool,
    pub hits: HitsBlock,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HitsBlock {
    pub total: TotalBlock,
    pub max_score: Option<f64>,
    pub hits: Vec<SearchHit>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TotalBlock {
    pub value: u64,
    pub relation: String,
}

pub fn execute(query: &Query, data: &IndexData, index_name: &str, size: usize, from: usize) -> SearchResponse {
    let started = std::time::Instant::now();
    let mut hits: Vec<SearchHit> = data
        .docs
        .values()
        .filter(|d| matches(query, d, data))
        .map(|d| SearchHit {
            index: index_name.into(),
            id: d.id.clone(),
            score: 1.0,
            source: d.source.clone(),
        })
        .collect();
    hits.sort_by(|a, b| a.id.cmp(&b.id));
    let total = hits.len() as u64;
    let paged: Vec<SearchHit> = hits.into_iter().skip(from).take(size).collect();
    SearchResponse {
        took: started.elapsed().as_millis() as u64,
        timed_out: false,
        hits: HitsBlock {
            total: TotalBlock {
                value: total,
                relation: "eq".into(),
            },
            max_score: if paged.is_empty() { None } else { Some(1.0) },
            hits: paged,
        },
    }
}

fn matches(query: &Query, doc: &StoredDoc, data: &IndexData) -> bool {
    match query {
        Query::MatchAll => true,
        Query::Term { field, value } => doc.source.get(field).map(|v| v == value).unwrap_or(false),
        Query::Terms { field, values } => doc
            .source
            .get(field)
            .map(|v| values.iter().any(|x| x == v))
            .unwrap_or(false),
        Query::Match { field, value } => {
            // For text fields, match any token; for keyword, exact match.
            let v = match doc.source.get(field).and_then(|v| v.as_str()) {
                Some(s) => s,
                None => return false,
            };
            let key = (field.clone(), value.to_lowercase());
            if let Some(ids) = data.inverted.get(&key) {
                if ids.contains(&doc.id) {
                    return true;
                }
            }
            v.eq_ignore_ascii_case(value)
        }
        Query::Range {
            field,
            gte,
            lte,
            gt,
            lt,
        } => match doc.source.get(field).and_then(|v| v.as_f64()) {
            None => false,
            Some(n) => {
                if let Some(b) = gte {
                    if n < *b {
                        return false;
                    }
                }
                if let Some(b) = gt {
                    if n <= *b {
                        return false;
                    }
                }
                if let Some(b) = lte {
                    if n > *b {
                        return false;
                    }
                }
                if let Some(b) = lt {
                    if n >= *b {
                        return false;
                    }
                }
                true
            }
        },
        Query::Exists { field } => doc
            .source
            .get(field)
            .map(|v| !v.is_null())
            .unwrap_or(false),
        Query::Prefix { field, value } => doc
            .source
            .get(field)
            .and_then(|v| v.as_str())
            .map(|s| s.starts_with(value))
            .unwrap_or(false),
        Query::Bool {
            must,
            should,
            must_not,
            filter,
            minimum_should_match,
        } => {
            for q in must {
                if !matches(q, doc, data) {
                    return false;
                }
            }
            for q in filter {
                if !matches(q, doc, data) {
                    return false;
                }
            }
            for q in must_not {
                if matches(q, doc, data) {
                    return false;
                }
            }
            if !should.is_empty() {
                let m = should.iter().filter(|q| matches(q, doc, data)).count();
                let need = minimum_should_match.unwrap_or(if must.is_empty() && filter.is_empty() {
                    1
                } else {
                    0
                });
                if m < need {
                    return false;
                }
            }
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::IndexRegistry;
    use crate::mapping::{FieldType, IndexMapping};
    use serde_json::json;

    fn setup() -> (IndexRegistry, &'static str) {
        let r = IndexRegistry::new();
        let mut m = IndexMapping::new();
        m.add_field("color", FieldType::Keyword);
        m.add_field("price", FieldType::Long);
        m.add_field("body", FieldType::Text);
        r.create("p", m).unwrap();
        r.index_doc("p", "1", json!({"color": "red", "price": 10, "body": "hello world"})).unwrap();
        r.index_doc("p", "2", json!({"color": "blue", "price": 20, "body": "goodbye"})).unwrap();
        r.index_doc("p", "3", json!({"color": "red", "price": 30, "body": "hello again"})).unwrap();
        (r, "p")
    }

    fn run_query(r: &IndexRegistry, name: &str, q: Query) -> SearchResponse {
        r.with_index(name, |d| execute(&q, d, name, 100, 0)).unwrap()
    }

    #[test]
    fn match_all_returns_all_docs() {
        let (r, n) = setup();
        let resp = run_query(&r, n, Query::MatchAll);
        assert_eq!(resp.hits.total.value, 3);
    }

    #[test]
    fn term_match_keyword() {
        let (r, n) = setup();
        let resp = run_query(
            &r,
            n,
            Query::Term {
                field: "color".into(),
                value: json!("red"),
            },
        );
        assert_eq!(resp.hits.total.value, 2);
    }

    #[test]
    fn terms_matches_any_in_list() {
        let (r, n) = setup();
        let resp = run_query(
            &r,
            n,
            Query::Terms {
                field: "color".into(),
                values: vec![json!("red"), json!("blue")],
            },
        );
        assert_eq!(resp.hits.total.value, 3);
    }

    #[test]
    fn range_gte_filters() {
        let (r, n) = setup();
        let resp = run_query(
            &r,
            n,
            Query::Range {
                field: "price".into(),
                gte: Some(20.0),
                lte: None,
                gt: None,
                lt: None,
            },
        );
        assert_eq!(resp.hits.total.value, 2);
    }

    #[test]
    fn range_gt_excludes_boundary() {
        let (r, n) = setup();
        let resp = run_query(
            &r,
            n,
            Query::Range {
                field: "price".into(),
                gte: None,
                lte: None,
                gt: Some(20.0),
                lt: None,
            },
        );
        assert_eq!(resp.hits.total.value, 1);
    }

    #[test]
    fn range_lt_excludes_boundary() {
        let (r, n) = setup();
        let resp = run_query(
            &r,
            n,
            Query::Range {
                field: "price".into(),
                gte: None,
                lte: None,
                gt: None,
                lt: Some(20.0),
            },
        );
        assert_eq!(resp.hits.total.value, 1);
    }

    #[test]
    fn exists_filters_present_field() {
        let (r, n) = setup();
        let resp = run_query(
            &r,
            n,
            Query::Exists {
                field: "color".into(),
            },
        );
        assert_eq!(resp.hits.total.value, 3);
    }

    #[test]
    fn prefix_matches_starting_substring() {
        let (r, n) = setup();
        let resp = run_query(
            &r,
            n,
            Query::Prefix {
                field: "color".into(),
                value: "re".into(),
            },
        );
        assert_eq!(resp.hits.total.value, 2);
    }

    #[test]
    fn match_uses_inverted_index_for_text() {
        let (r, n) = setup();
        let resp = run_query(
            &r,
            n,
            Query::Match {
                field: "body".into(),
                value: "hello".into(),
            },
        );
        assert_eq!(resp.hits.total.value, 2);
    }

    #[test]
    fn bool_must_combines() {
        let (r, n) = setup();
        let q = Query::Bool {
            must: vec![
                Query::Term {
                    field: "color".into(),
                    value: json!("red"),
                },
                Query::Range {
                    field: "price".into(),
                    gte: Some(20.0),
                    lte: None,
                    gt: None,
                    lt: None,
                },
            ],
            should: vec![],
            must_not: vec![],
            filter: vec![],
            minimum_should_match: None,
        };
        let resp = run_query(&r, n, q);
        assert_eq!(resp.hits.total.value, 1);
    }

    #[test]
    fn bool_must_not_excludes() {
        let (r, n) = setup();
        let q = Query::Bool {
            must: vec![],
            should: vec![],
            must_not: vec![Query::Term {
                field: "color".into(),
                value: json!("red"),
            }],
            filter: vec![],
            minimum_should_match: None,
        };
        let resp = run_query(&r, n, q);
        assert_eq!(resp.hits.total.value, 1);
    }

    #[test]
    fn bool_should_requires_one_match_when_no_must() {
        let (r, n) = setup();
        let q = Query::Bool {
            must: vec![],
            should: vec![
                Query::Term {
                    field: "color".into(),
                    value: json!("red"),
                },
                Query::Term {
                    field: "color".into(),
                    value: json!("blue"),
                },
            ],
            must_not: vec![],
            filter: vec![],
            minimum_should_match: None,
        };
        let resp = run_query(&r, n, q);
        assert_eq!(resp.hits.total.value, 3);
    }

    #[test]
    fn bool_minimum_should_match_two() {
        let (r, n) = setup();
        let q = Query::Bool {
            must: vec![],
            should: vec![
                Query::Term {
                    field: "color".into(),
                    value: json!("red"),
                },
                Query::Range {
                    field: "price".into(),
                    gte: Some(20.0),
                    lte: None,
                    gt: None,
                    lt: None,
                },
            ],
            must_not: vec![],
            filter: vec![],
            minimum_should_match: Some(2),
        };
        let resp = run_query(&r, n, q);
        assert_eq!(resp.hits.total.value, 1);
    }

    #[test]
    fn pagination_skips_then_takes() {
        let (r, n) = setup();
        let resp = r
            .with_index(n, |d| execute(&Query::MatchAll, d, n, 1, 1))
            .unwrap();
        assert_eq!(resp.hits.hits.len(), 1);
        assert_eq!(resp.hits.total.value, 3);
    }

    #[test]
    fn deserialize_match_query_from_json() {
        let s = r#"{"match":{"field":"body","value":"rust"}}"#;
        let q: Query = serde_json::from_str(s).unwrap();
        assert!(matches!(q, Query::Match { .. }));
    }

    #[test]
    fn deserialize_bool_query_from_json() {
        let s = r#"{"bool":{"must":[{"match_all":null}]}}"#;
        let q: Query = serde_json::from_str(s).unwrap();
        match q {
            Query::Bool { must, .. } => assert_eq!(must.len(), 1),
            other => panic!("expected Bool, got {:?}", other),
        }
    }
}

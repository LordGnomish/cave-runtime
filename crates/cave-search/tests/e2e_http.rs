//! End-to-end smoke tests: real reqwest HTTP client against the cave-search server.

use cave_search::server::{bind, AppState};
use serde_json::{json, Value};

async fn spawn() -> (String, tokio::task::JoinHandle<()>) {
    let state = AppState::new();
    let (addr, h) = bind(state, "127.0.0.1:0").await.unwrap();
    (format!("http://{}", addr), h)
}

#[tokio::test]
async fn cluster_health_returns_green_when_empty() {
    let (base, _h) = spawn().await;
    let r = reqwest::get(format!("{}/_cluster/health", base)).await.unwrap();
    assert_eq!(r.status(), 200);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["status"], "green");
}

#[tokio::test]
async fn create_index_then_get() {
    let (base, _h) = spawn().await;
    let client = reqwest::Client::new();
    let r = client
        .put(format!("{}/products", base))
        .json(&json!({"mappings":{"properties":{"name":{"type":"keyword"},"price":{"type":"long"}}}}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let g = client.get(format!("{}/products", base)).send().await.unwrap();
    assert_eq!(g.status(), 200);
    let v: Value = g.json().await.unwrap();
    assert!(v["products"]["mappings"]["properties"].is_object());
}

#[tokio::test]
async fn delete_index_returns_acknowledged() {
    let (base, _h) = spawn().await;
    let client = reqwest::Client::new();
    client.put(format!("{}/x", base)).send().await.unwrap();
    let r = client.delete(format!("{}/x", base)).send().await.unwrap();
    assert_eq!(r.status(), 200);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["acknowledged"], true);
}

#[tokio::test]
async fn put_doc_with_id_round_trip() {
    let (base, _h) = spawn().await;
    let client = reqwest::Client::new();
    client
        .put(format!("{}/idx", base))
        .json(&json!({"mappings":{"properties":{"name":{"type":"keyword"}}}}))
        .send().await.unwrap();
    let r = client
        .put(format!("{}/idx/_doc/1", base))
        .json(&json!({"name": "alice"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["_id"], "1");
    let g = client.get(format!("{}/idx/_doc/1", base)).send().await.unwrap();
    assert_eq!(g.status(), 200);
    let gv: Value = g.json().await.unwrap();
    assert_eq!(gv["found"], true);
    assert_eq!(gv["_source"]["name"], "alice");
}

#[tokio::test]
async fn post_doc_auto_assigns_id() {
    let (base, _h) = spawn().await;
    let client = reqwest::Client::new();
    client.put(format!("{}/idx", base)).send().await.unwrap();
    let r = client
        .post(format!("{}/idx/_doc", base))
        .json(&json!({"x": 1}))
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    assert!(v["_id"].as_str().unwrap().len() >= 32);
}

#[tokio::test]
async fn delete_doc_returns_deleted() {
    let (base, _h) = spawn().await;
    let client = reqwest::Client::new();
    client.put(format!("{}/idx", base)).send().await.unwrap();
    client.put(format!("{}/idx/_doc/1", base)).json(&json!({"x":1})).send().await.unwrap();
    let r = client.delete(format!("{}/idx/_doc/1", base)).send().await.unwrap();
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["result"], "deleted");
    let g = client.get(format!("{}/idx/_doc/1", base)).send().await.unwrap();
    assert_eq!(g.status(), 404);
}

#[tokio::test]
async fn search_match_all() {
    let (base, _h) = spawn().await;
    let client = reqwest::Client::new();
    client.put(format!("{}/idx", base)).send().await.unwrap();
    for (id, color) in [("1","red"),("2","blue"),("3","red")] {
        client.put(format!("{}/idx/_doc/{}", base, id))
            .json(&json!({"color": color}))
            .send().await.unwrap();
    }
    let r = client
        .post(format!("{}/idx/_search", base))
        .json(&json!({"query": {"match_all": null}}))
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["hits"]["total"]["value"], 3);
}

#[tokio::test]
async fn search_term_query_filters() {
    let (base, _h) = spawn().await;
    let client = reqwest::Client::new();
    client
        .put(format!("{}/idx", base))
        .json(&json!({"mappings":{"properties":{"color":{"type":"keyword"}}}}))
        .send().await.unwrap();
    for (id, color) in [("1","red"),("2","blue"),("3","red")] {
        client.put(format!("{}/idx/_doc/{}", base, id))
            .json(&json!({"color": color}))
            .send().await.unwrap();
    }
    let r = client
        .post(format!("{}/idx/_search", base))
        .json(&json!({"query": {"term": {"field": "color", "value": "red"}}}))
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["hits"]["total"]["value"], 2);
}

#[tokio::test]
async fn search_aggregation_terms() {
    let (base, _h) = spawn().await;
    let client = reqwest::Client::new();
    client
        .put(format!("{}/idx", base))
        .json(&json!({"mappings":{"properties":{"color":{"type":"keyword"}}}}))
        .send().await.unwrap();
    for (id, color) in [("1","red"),("2","blue"),("3","red"),("4","red")] {
        client.put(format!("{}/idx/_doc/{}", base, id))
            .json(&json!({"color": color}))
            .send().await.unwrap();
    }
    let r = client
        .post(format!("{}/idx/_search", base))
        .json(&json!({
            "size": 0,
            "aggs": {
                "by_color": {"terms": {"field": "color"}}
            }
        }))
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    let buckets = v["aggregations"]["by_color"]["buckets"].as_array().unwrap();
    assert_eq!(buckets[0]["key"], "red");
    assert_eq!(buckets[0]["doc_count"], 3);
}

#[tokio::test]
async fn search_range_query() {
    let (base, _h) = spawn().await;
    let client = reqwest::Client::new();
    client
        .put(format!("{}/idx", base))
        .json(&json!({"mappings":{"properties":{"price":{"type":"long"}}}}))
        .send().await.unwrap();
    for (id, p) in [(1u32,10),(2,20),(3,30)] {
        client.put(format!("{}/idx/_doc/{}", base, id))
            .json(&json!({"price": p}))
            .send().await.unwrap();
    }
    let r = client
        .post(format!("{}/idx/_search", base))
        .json(&json!({"query": {"range": {"field": "price", "gte": 20.0}}}))
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["hits"]["total"]["value"], 2);
}

#[tokio::test]
async fn search_bool_must_combines() {
    let (base, _h) = spawn().await;
    let client = reqwest::Client::new();
    client
        .put(format!("{}/idx", base))
        .json(&json!({"mappings":{"properties":{"color":{"type":"keyword"},"price":{"type":"long"}}}}))
        .send().await.unwrap();
    for (id, color, p) in [("1","red",10),("2","red",30),("3","blue",30)] {
        client.put(format!("{}/idx/_doc/{}", base, id))
            .json(&json!({"color": color, "price": p}))
            .send().await.unwrap();
    }
    let r = client
        .post(format!("{}/idx/_search", base))
        .json(&json!({
            "query": {
                "bool": {
                    "must": [
                        {"term": {"field": "color", "value": "red"}},
                        {"range": {"field": "price", "gte": 20.0}}
                    ]
                }
            }
        }))
        .send().await.unwrap();
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["hits"]["total"]["value"], 1);
}

#[tokio::test]
async fn search_pagination_with_size_and_from() {
    let (base, _h) = spawn().await;
    let client = reqwest::Client::new();
    client.put(format!("{}/idx", base)).send().await.unwrap();
    for i in 1..=5 {
        client.put(format!("{}/idx/_doc/{}", base, i))
            .json(&json!({"n": i}))
            .send().await.unwrap();
    }
    let r = client
        .post(format!("{}/idx/_search", base))
        .json(&json!({"query": {"match_all": null}, "size": 2, "from": 1}))
        .send().await.unwrap();
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["hits"]["hits"].as_array().unwrap().len(), 2);
    assert_eq!(v["hits"]["total"]["value"], 5);
}

#[tokio::test]
async fn cat_indices_returns_array() {
    let (base, _h) = spawn().await;
    let client = reqwest::Client::new();
    client.put(format!("{}/a", base)).send().await.unwrap();
    client.put(format!("{}/b", base)).send().await.unwrap();
    let r = client.get(format!("{}/_cat/indices", base)).send().await.unwrap();
    let v: Value = r.json().await.unwrap();
    let arr = v.as_array().unwrap();
    assert_eq!(arr.len(), 2);
}

#[tokio::test]
async fn create_duplicate_index_returns_400() {
    let (base, _h) = spawn().await;
    let client = reqwest::Client::new();
    client.put(format!("{}/x", base)).send().await.unwrap();
    let r = client.put(format!("{}/x", base)).send().await.unwrap();
    assert_eq!(r.status(), 400);
}

#[tokio::test]
async fn get_unknown_doc_returns_404() {
    let (base, _h) = spawn().await;
    let client = reqwest::Client::new();
    client.put(format!("{}/idx", base)).send().await.unwrap();
    let r = client.get(format!("{}/idx/_doc/missing", base)).send().await.unwrap();
    assert_eq!(r.status(), 404);
}

#[tokio::test]
async fn put_doc_invalid_type_returns_400() {
    let (base, _h) = spawn().await;
    let client = reqwest::Client::new();
    client
        .put(format!("{}/idx", base))
        .json(&json!({"mappings":{"properties":{"n":{"type":"long"}}}}))
        .send().await.unwrap();
    let r = client.put(format!("{}/idx/_doc/1", base))
        .json(&json!({"n": "string"}))
        .send().await.unwrap();
    assert_eq!(r.status(), 400);
}

#[tokio::test]
async fn re_put_increments_version() {
    let (base, _h) = spawn().await;
    let client = reqwest::Client::new();
    client.put(format!("{}/idx", base)).send().await.unwrap();
    client.put(format!("{}/idx/_doc/1", base)).json(&json!({"x":1})).send().await.unwrap();
    let r = client.put(format!("{}/idx/_doc/1", base)).json(&json!({"x":2})).send().await.unwrap();
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["_version"], 2);
    assert_eq!(v["result"], "updated");
}

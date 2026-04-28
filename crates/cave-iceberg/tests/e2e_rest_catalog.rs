//! End-to-end smoke tests: real reqwest HTTP client against the cave-iceberg
//! REST catalog server.

use cave_iceberg::server::{bind, AppState};
use serde_json::{json, Value};

async fn spawn() -> (String, tokio::task::JoinHandle<()>) {
    let state = AppState::new();
    let (addr, h) = bind(state, "127.0.0.1:0").await.unwrap();
    (format!("http://{}", addr), h)
}

#[tokio::test]
async fn config_endpoint_returns_overrides_and_defaults() {
    let (base, _h) = spawn().await;
    let r = reqwest::get(format!("{}/v1/config", base)).await.unwrap();
    assert_eq!(r.status(), 200);
    let v: Value = r.json().await.unwrap();
    assert!(v.get("overrides").is_some());
    assert!(v.get("defaults").is_some());
}

#[tokio::test]
async fn create_namespace_and_load() {
    let (base, _h) = spawn().await;
    let client = reqwest::Client::new();
    let r = client
        .post(format!("{}/v1/namespaces", base))
        .json(&json!({"namespace": ["acme"], "properties": {"owner": "alice"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let g = client
        .get(format!("{}/v1/namespaces/acme", base))
        .send()
        .await
        .unwrap();
    let v: Value = g.json().await.unwrap();
    assert_eq!(v["properties"]["owner"], "alice");
}

#[tokio::test]
async fn create_namespace_duplicate_returns_409() {
    let (base, _h) = spawn().await;
    let client = reqwest::Client::new();
    client
        .post(format!("{}/v1/namespaces", base))
        .json(&json!({"namespace": ["acme"]}))
        .send()
        .await
        .unwrap();
    let r = client
        .post(format!("{}/v1/namespaces", base))
        .json(&json!({"namespace": ["acme"]}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 409);
}

#[tokio::test]
async fn drop_namespace_returns_204() {
    let (base, _h) = spawn().await;
    let client = reqwest::Client::new();
    client
        .post(format!("{}/v1/namespaces", base))
        .json(&json!({"namespace": ["x"]}))
        .send()
        .await
        .unwrap();
    let r = client
        .delete(format!("{}/v1/namespaces/x", base))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
}

#[tokio::test]
async fn list_namespaces_returns_top_level_only() {
    let (base, _h) = spawn().await;
    let client = reqwest::Client::new();
    for n in ["acme", "acme.warehouse", "other"] {
        client
            .post(format!("{}/v1/namespaces", base))
            .json(&json!({"namespace": n.split('.').collect::<Vec<&str>>()}))
            .send()
            .await
            .unwrap();
    }
    let r = client.get(format!("{}/v1/namespaces", base)).send().await.unwrap();
    let v: Value = r.json().await.unwrap();
    let names: Vec<String> = v["namespaces"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_array().unwrap()[0].as_str().unwrap().into())
        .collect();
    assert!(names.contains(&"acme".into()));
    assert!(names.contains(&"other".into()));
}

#[tokio::test]
async fn create_table_then_load() {
    let (base, _h) = spawn().await;
    let client = reqwest::Client::new();
    client
        .post(format!("{}/v1/namespaces", base))
        .json(&json!({"namespace": ["acme"]}))
        .send()
        .await
        .unwrap();
    let r = client
        .post(format!("{}/v1/namespaces/acme/tables", base))
        .json(&json!({"name": "users", "metadata_location": "v1.metadata.json"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let g = client
        .get(format!("{}/v1/namespaces/acme/tables/users", base))
        .send()
        .await
        .unwrap();
    let v: Value = g.json().await.unwrap();
    assert_eq!(v["name"], "users");
    assert_eq!(v["metadata_location"], "v1.metadata.json");
}

#[tokio::test]
async fn drop_table_returns_204() {
    let (base, _h) = spawn().await;
    let client = reqwest::Client::new();
    client
        .post(format!("{}/v1/namespaces", base))
        .json(&json!({"namespace": ["acme"]}))
        .send()
        .await
        .unwrap();
    client
        .post(format!("{}/v1/namespaces/acme/tables", base))
        .json(&json!({"name": "t", "metadata_location": "v1"}))
        .send()
        .await
        .unwrap();
    let r = client
        .delete(format!("{}/v1/namespaces/acme/tables/t", base))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
}

#[tokio::test]
async fn commit_with_matching_expected_succeeds() {
    let (base, _h) = spawn().await;
    let client = reqwest::Client::new();
    client
        .post(format!("{}/v1/namespaces", base))
        .json(&json!({"namespace": ["acme"]}))
        .send()
        .await
        .unwrap();
    client
        .post(format!("{}/v1/namespaces/acme/tables", base))
        .json(&json!({"name": "t", "metadata_location": "v1.json"}))
        .send()
        .await
        .unwrap();
    let r = client
        .post(format!("{}/v1/namespaces/acme/tables/t", base))
        .json(&json!({
            "expected_metadata_location": "v1.json",
            "new_metadata_location": "v2.json",
            "new_snapshot_id": 100
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["metadata_location"], "v2.json");
    assert_eq!(v["current_snapshot_id"], 100);
}

#[tokio::test]
async fn commit_conflict_returns_409() {
    let (base, _h) = spawn().await;
    let client = reqwest::Client::new();
    client
        .post(format!("{}/v1/namespaces", base))
        .json(&json!({"namespace": ["acme"]}))
        .send()
        .await
        .unwrap();
    client
        .post(format!("{}/v1/namespaces/acme/tables", base))
        .json(&json!({"name": "t", "metadata_location": "v1.json"}))
        .send()
        .await
        .unwrap();
    // First commit succeeds.
    client
        .post(format!("{}/v1/namespaces/acme/tables/t", base))
        .json(&json!({
            "expected_metadata_location": "v1.json",
            "new_metadata_location": "v2.json",
            "new_snapshot_id": 100
        }))
        .send()
        .await
        .unwrap();
    // Stale committer races and loses.
    let r = client
        .post(format!("{}/v1/namespaces/acme/tables/t", base))
        .json(&json!({
            "expected_metadata_location": "v1.json",
            "new_metadata_location": "v3.json",
            "new_snapshot_id": 200
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 409);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["error"]["type"], "CommitFailedException");
}

#[tokio::test]
async fn rename_table_endpoint() {
    let (base, _h) = spawn().await;
    let client = reqwest::Client::new();
    client
        .post(format!("{}/v1/namespaces", base))
        .json(&json!({"namespace": ["acme"]}))
        .send()
        .await
        .unwrap();
    client
        .post(format!("{}/v1/namespaces/acme/tables", base))
        .json(&json!({"name": "old", "metadata_location": "v1"}))
        .send()
        .await
        .unwrap();
    let r = client
        .post(format!("{}/v1/tables/rename", base))
        .json(&json!({
            "source": {"namespace": ["acme"], "name": "old"},
            "destination": {"namespace": ["acme"], "name": "new"}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    let g = client
        .get(format!("{}/v1/namespaces/acme/tables/new", base))
        .send()
        .await
        .unwrap();
    assert_eq!(g.status(), 200);
}

#[tokio::test]
async fn list_tables_returns_identifiers() {
    let (base, _h) = spawn().await;
    let client = reqwest::Client::new();
    client
        .post(format!("{}/v1/namespaces", base))
        .json(&json!({"namespace": ["acme"]}))
        .send()
        .await
        .unwrap();
    client
        .post(format!("{}/v1/namespaces/acme/tables", base))
        .json(&json!({"name": "a", "metadata_location": "v1"}))
        .send()
        .await
        .unwrap();
    client
        .post(format!("{}/v1/namespaces/acme/tables", base))
        .json(&json!({"name": "b", "metadata_location": "v2"}))
        .send()
        .await
        .unwrap();
    let r = client
        .get(format!("{}/v1/namespaces/acme/tables", base))
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    let names: Vec<String> = v["identifiers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().into())
        .collect();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(sorted, vec!["a", "b"]);
}

#[tokio::test]
async fn drop_namespace_with_tables_returns_409() {
    let (base, _h) = spawn().await;
    let client = reqwest::Client::new();
    client
        .post(format!("{}/v1/namespaces", base))
        .json(&json!({"namespace": ["acme"]}))
        .send()
        .await
        .unwrap();
    client
        .post(format!("{}/v1/namespaces/acme/tables", base))
        .json(&json!({"name": "t", "metadata_location": "v1"}))
        .send()
        .await
        .unwrap();
    let r = client
        .delete(format!("{}/v1/namespaces/acme", base))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 409);
}

#[tokio::test]
async fn load_unknown_table_returns_404() {
    let (base, _h) = spawn().await;
    let client = reqwest::Client::new();
    client
        .post(format!("{}/v1/namespaces", base))
        .json(&json!({"namespace": ["acme"]}))
        .send()
        .await
        .unwrap();
    let r = client
        .get(format!("{}/v1/namespaces/acme/tables/missing", base))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
}

#[tokio::test]
async fn update_namespace_properties_round_trip() {
    let (base, _h) = spawn().await;
    let client = reqwest::Client::new();
    client
        .post(format!("{}/v1/namespaces", base))
        .json(&json!({"namespace": ["acme"]}))
        .send()
        .await
        .unwrap();
    let r = client
        .post(format!("{}/v1/namespaces/acme/properties", base))
        .json(&json!({"updates": {"k1": "v1"}, "removals": []}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let g = client
        .get(format!("{}/v1/namespaces/acme", base))
        .send()
        .await
        .unwrap();
    let v: Value = g.json().await.unwrap();
    assert_eq!(v["properties"]["k1"], "v1");
}

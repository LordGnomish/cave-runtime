//! End-to-end smoke tests: real HTTP client against the cave-blob axum server.
//! Verifies the S3 surface (bucket/object/multipart) is wire-faithful.

use cave_blob::server::{bind, AppState};

async fn spawn() -> (String, tokio::task::JoinHandle<()>) {
    let state = AppState::new();
    let (addr, handle) = bind(state, "127.0.0.1:0").await.unwrap();
    (format!("http://{}", addr), handle)
}

#[tokio::test]
async fn create_bucket_then_head() {
    let (base, _h) = spawn().await;
    let client = reqwest::Client::new();
    let r = client.put(format!("{}/my-bucket", base)).send().await.unwrap();
    assert_eq!(r.status(), 200);
    let head = client.head(format!("{}/my-bucket", base)).send().await.unwrap();
    assert_eq!(head.status(), 200);
}

#[tokio::test]
async fn create_then_delete_bucket() {
    let (base, _h) = spawn().await;
    let client = reqwest::Client::new();
    client.put(format!("{}/buk1", base)).send().await.unwrap();
    let r = client.delete(format!("{}/buk1", base)).send().await.unwrap();
    assert_eq!(r.status(), 204);
}

#[tokio::test]
async fn create_bucket_twice_conflict() {
    let (base, _h) = spawn().await;
    let client = reqwest::Client::new();
    client.put(format!("{}/buk2", base)).send().await.unwrap();
    let r = client.put(format!("{}/buk2", base)).send().await.unwrap();
    assert_eq!(r.status(), 409);
}

#[tokio::test]
async fn put_then_get_object_round_trip() {
    let (base, _h) = spawn().await;
    let client = reqwest::Client::new();
    client.put(format!("{}/buk3", base)).send().await.unwrap();
    let r = client
        .put(format!("{}/buk3/hello.txt", base))
        .header("content-type", "text/plain")
        .body("hello world")
        .send()
        .await
        .unwrap();
    let status = r.status();
    let etag_present = r.headers().get("etag").is_some();
    let body = r.text().await.unwrap();
    assert_eq!(status, 200, "response body: {}", body);
    assert!(etag_present);

    let g = client.get(format!("{}/buk3/hello.txt", base)).send().await.unwrap();
    assert_eq!(g.status(), 200);
    assert_eq!(g.headers().get("content-type").unwrap(), "text/plain");
    let body = g.bytes().await.unwrap();
    assert_eq!(body.as_ref(), b"hello world");
}

#[tokio::test]
async fn head_object_returns_metadata() {
    let (base, _h) = spawn().await;
    let client = reqwest::Client::new();
    client.put(format!("{}/buk4", base)).send().await.unwrap();
    client
        .put(format!("{}/buk4/k", base))
        .body("x".repeat(123))
        .send()
        .await
        .unwrap();
    let h = client.head(format!("{}/buk4/k", base)).send().await.unwrap();
    assert_eq!(h.status(), 200);
    assert_eq!(
        h.headers().get("content-length").unwrap().to_str().unwrap(),
        "123"
    );
}

#[tokio::test]
async fn delete_then_get_returns_404() {
    let (base, _h) = spawn().await;
    let client = reqwest::Client::new();
    client.put(format!("{}/buk5", base)).send().await.unwrap();
    client.put(format!("{}/buk5/k", base)).body("x").send().await.unwrap();
    client.delete(format!("{}/buk5/k", base)).send().await.unwrap();
    let g = client.get(format!("{}/buk5/k", base)).send().await.unwrap();
    assert_eq!(g.status(), 404);
}

#[tokio::test]
async fn list_objects_with_prefix() {
    let (base, _h) = spawn().await;
    let client = reqwest::Client::new();
    client.put(format!("{}/buk6", base)).send().await.unwrap();
    for k in ["logs/a.txt", "logs/b.txt", "data/c.txt"] {
        client
            .put(format!("{}/buk6/{}", base, k))
            .body("x")
            .send()
            .await
            .unwrap();
    }
    let r = client
        .get(format!("{}/buk6?prefix=logs/", base))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let body = r.text().await.unwrap();
    assert!(body.contains("<Key>logs/a.txt</Key>"));
    assert!(body.contains("<Key>logs/b.txt</Key>"));
    assert!(!body.contains("<Key>data/c.txt</Key>"));
}

#[tokio::test]
async fn multipart_upload_round_trip() {
    let (base, _h) = spawn().await;
    let client = reqwest::Client::new();
    client.put(format!("{}/buk7", base)).send().await.unwrap();
    let init = client
        .post(format!("{}/buk7/big.bin?uploads", base))
        .send()
        .await
        .unwrap();
    let init_body = init.text().await.unwrap();
    let upload_id = extract_xml_field(&init_body, "UploadId").unwrap();

    // Upload 2 parts.
    let p1 = client
        .put(format!(
            "{}/buk7/big.bin?uploadId={}&partNumber=1",
            base, upload_id
        ))
        .body("partA")
        .send()
        .await
        .unwrap();
    let etag1 = strip_quotes(p1.headers().get("etag").unwrap().to_str().unwrap());

    let p2 = client
        .put(format!(
            "{}/buk7/big.bin?uploadId={}&partNumber=2",
            base, upload_id
        ))
        .body("partB")
        .send()
        .await
        .unwrap();
    let etag2 = strip_quotes(p2.headers().get("etag").unwrap().to_str().unwrap());

    let complete_body = format!(
        "<CompleteMultipartUpload>\
         <Part><PartNumber>1</PartNumber><ETag>\"{}\"</ETag></Part>\
         <Part><PartNumber>2</PartNumber><ETag>\"{}\"</ETag></Part>\
         </CompleteMultipartUpload>",
        etag1, etag2
    );
    let c = client
        .post(format!("{}/buk7/big.bin?uploadId={}", base, upload_id))
        .body(complete_body)
        .send()
        .await
        .unwrap();
    assert_eq!(c.status(), 200);

    let g = client.get(format!("{}/buk7/big.bin", base)).send().await.unwrap();
    let body = g.bytes().await.unwrap();
    assert_eq!(body.as_ref(), b"partApartB");
}

#[tokio::test]
async fn multipart_abort_drops_upload() {
    let (base, _h) = spawn().await;
    let client = reqwest::Client::new();
    client.put(format!("{}/buk8", base)).send().await.unwrap();
    let init = client
        .post(format!("{}/buk8/k?uploads", base))
        .send()
        .await
        .unwrap();
    let init_body = init.text().await.unwrap();
    let upload_id = extract_xml_field(&init_body, "UploadId").unwrap();
    let r = client
        .delete(format!("{}/buk8/k?uploadId={}", base, upload_id))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    // Subsequent abort should fail (404).
    let r2 = client
        .delete(format!("{}/buk8/k?uploadId={}", base, upload_id))
        .send()
        .await
        .unwrap();
    assert_eq!(r2.status(), 404);
}

#[tokio::test]
async fn delete_unknown_bucket_returns_404() {
    let (base, _h) = spawn().await;
    let client = reqwest::Client::new();
    let r = client.delete(format!("{}/missing", base)).send().await.unwrap();
    assert_eq!(r.status(), 404);
}

#[tokio::test]
async fn list_unknown_bucket_returns_404() {
    let (base, _h) = spawn().await;
    let client = reqwest::Client::new();
    let r = client.get(format!("{}/missing", base)).send().await.unwrap();
    assert_eq!(r.status(), 404);
}

#[tokio::test]
async fn put_object_to_unknown_bucket_returns_404() {
    let (base, _h) = spawn().await;
    let client = reqwest::Client::new();
    let r = client
        .put(format!("{}/missing/k", base))
        .body("x")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
}

#[tokio::test]
async fn get_object_etag_in_quotes() {
    let (base, _h) = spawn().await;
    let client = reqwest::Client::new();
    client.put(format!("{}/buk9", base)).send().await.unwrap();
    let r = client
        .put(format!("{}/buk9/k", base))
        .body("hello")
        .send()
        .await
        .unwrap();
    let etag = r.headers().get("etag").unwrap().to_str().unwrap();
    assert!(etag.starts_with('"') && etag.ends_with('"'));
}

#[tokio::test]
async fn put_with_object_lock_governance_header() {
    let (base, _h) = spawn().await;
    let client = reqwest::Client::new();
    client.put(format!("{}/buk10", base)).send().await.unwrap();
    let until = (chrono::Utc::now() + chrono::Duration::days(7)).to_rfc3339();
    let r = client
        .put(format!("{}/buk10/locked.txt", base))
        .header("x-amz-object-lock-mode", "GOVERNANCE")
        .header("x-amz-object-lock-retain-until-date", &until)
        .body("x")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    // Delete should be blocked by Immutable.
    let d = client.delete(format!("{}/buk10/locked.txt", base)).send().await.unwrap();
    assert_eq!(d.status(), 403);
}

fn extract_xml_field(s: &str, tag: &str) -> Option<String> {
    let open = format!("<{}>", tag);
    let close = format!("</{}>", tag);
    let start = s.find(&open)?;
    let end = s.find(&close)?;
    Some(s[start + open.len()..end].to_string())
}

fn strip_quotes(s: &str) -> String {
    s.trim_matches('"').to_string()
}

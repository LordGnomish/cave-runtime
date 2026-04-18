//! JSON request/response models for the HTTP admin API.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Serialize, Deserialize)]
pub struct FindRequest {
    pub filter: Option<Value>,
    pub projection: Option<Value>,
    pub limit: Option<i32>,
    pub skip: Option<i32>,
    pub sort: Option<Value>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct InsertRequest {
    pub documents: Vec<Value>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UpdateRequest {
    pub filter: Option<Value>,
    pub update: Value,
    pub multi: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DeleteRequest {
    pub filter: Option<Value>,
    pub multi: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AggregateRequest {
    pub pipeline: Vec<Value>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct IndexCreateRequest {
    pub keys: serde_json::Map<String, Value>,
    pub unique: Option<bool>,
    pub name: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FindResponse {
    pub documents: Vec<Value>,
    pub count: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct InsertResponse {
    pub inserted_ids: Vec<String>,
    pub inserted_count: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UpdateResponse {
    pub modified_count: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DeleteResponse {
    pub deleted_count: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CollectionStats {
    pub name: String,
    pub document_count: u64,
    pub index_count: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ServerInfo {
    pub version: String,
    pub pid: u32,
    pub uptime_seconds: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HealthResponse {
    pub status: String,
}

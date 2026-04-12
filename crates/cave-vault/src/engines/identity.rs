use crate::error::{VaultError, VaultResult};
use crate::response::VaultResponse;
use crate::VaultState;
use axum::{
    extract::{Json, Path, State},
    http::HeaderMap,
    routing::{delete, get, post},
    Router,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

fn extract_token(headers: &HeaderMap) -> VaultResult<String> {
    headers.get("x-vault-token")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .ok_or(VaultError::BadToken)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entity {
    pub id: String,
    pub name: String,
    pub metadata: HashMap<String, String>,
    pub policies: Vec<String>,
    pub aliases: Vec<EntityAlias>,
    pub disabled: bool,
    pub creation_time: String,
    pub last_update_time: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityAlias {
    pub id: String,
    pub canonical_id: String,
    pub mount_accessor: String,
    pub mount_type: String,
    pub name: String,
    pub metadata: HashMap<String, String>,
    pub creation_time: String,
    pub last_update_time: String,
    pub merged_from_canonical_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Group {
    pub id: String,
    pub name: String,
    pub group_type: String, // "internal" or "external"
    pub policies: Vec<String>,
    pub member_entity_ids: Vec<String>,
    pub member_group_ids: Vec<String>,
    pub metadata: HashMap<String, String>,
    pub aliases: Vec<GroupAlias>,
    pub creation_time: String,
    pub last_update_time: String,
    pub parent_group_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupAlias {
    pub id: String,
    pub canonical_id: String,
    pub mount_accessor: String,
    pub mount_type: String,
    pub name: String,
    pub creation_time: String,
    pub last_update_time: String,
}

#[derive(Default)]
pub struct IdentityStore {
    pub entities: HashMap<String, Entity>,   // id -> entity
    pub entity_names: HashMap<String, String>, // name -> id
    pub groups: HashMap<String, Group>,       // id -> group
    pub group_names: HashMap<String, String>,  // name -> id
    pub entity_aliases: HashMap<String, EntityAlias>, // alias_id -> alias
    pub group_aliases: HashMap<String, GroupAlias>,   // alias_id -> alias
}

// Entity CRUD
pub async fn create_entity(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let id = Uuid::new_v4().to_string();
    let name = body.get("name")
        .and_then(|v| v.as_str())
        .unwrap_or(&id)
        .to_string();

    let entity = Entity {
        id: id.clone(),
        name: name.clone(),
        metadata: body.get("metadata").and_then(|v| v.as_object())
            .map(|m| m.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect())
            .unwrap_or_default(),
        policies: body.get("policies").and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
            .unwrap_or_default(),
        aliases: Vec::new(),
        disabled: body.get("disabled").and_then(|v| v.as_bool()).unwrap_or(false),
        creation_time: Utc::now().to_rfc3339(),
        last_update_time: Utc::now().to_rfc3339(),
    };

    let mut store = state.identity_store.write().await;
    store.entity_names.insert(name, id.clone());
    store.entities.insert(id.clone(), entity);

    Ok(VaultResponse::new().with_data(json!({ "id": id })))
}

pub async fn read_entity_by_id(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let store = state.identity_store.read().await;
    let entity = store.entities.get(&id)
        .ok_or_else(|| VaultError::NotFound(format!("entity {} not found", id)))?;
    Ok(VaultResponse::new().with_data(serde_json::to_value(entity).unwrap_or_default()))
}

pub async fn read_entity_by_name(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let store = state.identity_store.read().await;
    let id = store.entity_names.get(&name)
        .ok_or_else(|| VaultError::NotFound(format!("entity {} not found", name)))?;
    let entity = store.entities.get(id)
        .ok_or_else(|| VaultError::NotFound(format!("entity {} not found", name)))?;
    Ok(VaultResponse::new().with_data(serde_json::to_value(entity).unwrap_or_default()))
}

pub async fn update_entity(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let mut store = state.identity_store.write().await;
    let entity = store.entities.get_mut(&id)
        .ok_or_else(|| VaultError::NotFound(format!("entity {} not found", id)))?;
    if let Some(name) = body.get("name").and_then(|v| v.as_str()) {
        entity.name = name.to_string();
    }
    if let Some(meta) = body.get("metadata").and_then(|v| v.as_object()) {
        entity.metadata = meta.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect();
    }
    if let Some(policies) = body.get("policies").and_then(|v| v.as_array()) {
        entity.policies = policies.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect();
    }
    if let Some(disabled) = body.get("disabled").and_then(|v| v.as_bool()) {
        entity.disabled = disabled;
    }
    entity.last_update_time = Utc::now().to_rfc3339();
    Ok(VaultResponse::new().with_data(json!({ "id": id })))
}

pub async fn delete_entity(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let mut store = state.identity_store.write().await;
    if let Some(entity) = store.entities.remove(&id) {
        store.entity_names.remove(&entity.name);
    }
    Ok(VaultResponse::new())
}

pub async fn list_entities(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let store = state.identity_store.read().await;
    let keys: Vec<String> = store.entities.keys().cloned().collect();
    Ok(VaultResponse::new().with_data(json!({ "keys": keys })))
}

// Entity Alias CRUD
pub async fn create_entity_alias(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let alias_id = Uuid::new_v4().to_string();
    let canonical_id = body.get("canonical_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| VaultError::InvalidRequest("canonical_id required".into()))?
        .to_string();
    let name = body.get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| VaultError::InvalidRequest("name required".into()))?
        .to_string();
    let mount_accessor = body.get("mount_accessor")
        .and_then(|v| v.as_str())
        .unwrap_or("").to_string();

    let alias = EntityAlias {
        id: alias_id.clone(),
        canonical_id: canonical_id.clone(),
        mount_accessor,
        mount_type: String::new(),
        name,
        metadata: HashMap::new(),
        creation_time: Utc::now().to_rfc3339(),
        last_update_time: Utc::now().to_rfc3339(),
        merged_from_canonical_ids: Vec::new(),
    };

    let mut store = state.identity_store.write().await;
    // Add alias to entity
    if let Some(entity) = store.entities.get_mut(&canonical_id) {
        entity.aliases.push(alias.clone());
    }
    store.entity_aliases.insert(alias_id.clone(), alias);

    Ok(VaultResponse::new().with_data(json!({ "id": alias_id, "canonical_id": canonical_id })))
}

pub async fn read_entity_alias(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let store = state.identity_store.read().await;
    let alias = store.entity_aliases.get(&id)
        .ok_or_else(|| VaultError::NotFound(format!("alias {} not found", id)))?;
    Ok(VaultResponse::new().with_data(serde_json::to_value(alias).unwrap_or_default()))
}

pub async fn delete_entity_alias(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let mut store = state.identity_store.write().await;
    store.entity_aliases.remove(&id);
    Ok(VaultResponse::new())
}

pub async fn list_entity_aliases(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let store = state.identity_store.read().await;
    let keys: Vec<String> = store.entity_aliases.keys().cloned().collect();
    Ok(VaultResponse::new().with_data(json!({ "keys": keys })))
}

// Group CRUD
pub async fn create_group(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let id = Uuid::new_v4().to_string();
    let name = body.get("name")
        .and_then(|v| v.as_str())
        .unwrap_or(&id)
        .to_string();

    let group = Group {
        id: id.clone(),
        name: name.clone(),
        group_type: body.get("type").and_then(|v| v.as_str()).unwrap_or("internal").to_string(),
        policies: body.get("policies").and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
            .unwrap_or_default(),
        member_entity_ids: body.get("member_entity_ids").and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
            .unwrap_or_default(),
        member_group_ids: body.get("member_group_ids").and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
            .unwrap_or_default(),
        metadata: body.get("metadata").and_then(|v| v.as_object())
            .map(|m| m.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect())
            .unwrap_or_default(),
        aliases: Vec::new(),
        creation_time: Utc::now().to_rfc3339(),
        last_update_time: Utc::now().to_rfc3339(),
        parent_group_ids: Vec::new(),
    };

    let mut store = state.identity_store.write().await;
    store.group_names.insert(name, id.clone());
    store.groups.insert(id.clone(), group);

    Ok(VaultResponse::new().with_data(json!({ "id": id })))
}

pub async fn read_group_by_id(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let store = state.identity_store.read().await;
    let group = store.groups.get(&id)
        .ok_or_else(|| VaultError::NotFound(format!("group {} not found", id)))?;
    Ok(VaultResponse::new().with_data(serde_json::to_value(group).unwrap_or_default()))
}

pub async fn read_group_by_name(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let store = state.identity_store.read().await;
    let id = store.group_names.get(&name)
        .ok_or_else(|| VaultError::NotFound(format!("group {} not found", name)))?;
    let group = store.groups.get(id)
        .ok_or_else(|| VaultError::NotFound(format!("group {} not found", name)))?;
    Ok(VaultResponse::new().with_data(serde_json::to_value(group).unwrap_or_default()))
}

pub async fn delete_group(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let mut store = state.identity_store.write().await;
    if let Some(group) = store.groups.remove(&id) {
        store.group_names.remove(&group.name);
    }
    Ok(VaultResponse::new())
}

pub async fn list_groups(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let store = state.identity_store.read().await;
    let keys: Vec<String> = store.groups.keys().cloned().collect();
    Ok(VaultResponse::new().with_data(json!({ "keys": keys })))
}

// Group Alias CRUD
pub async fn create_group_alias(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let alias_id = Uuid::new_v4().to_string();
    let canonical_id = body.get("canonical_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| VaultError::InvalidRequest("canonical_id required".into()))?
        .to_string();
    let name = body.get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| VaultError::InvalidRequest("name required".into()))?
        .to_string();

    let alias = GroupAlias {
        id: alias_id.clone(),
        canonical_id: canonical_id.clone(),
        mount_accessor: body.get("mount_accessor").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        mount_type: String::new(),
        name,
        creation_time: Utc::now().to_rfc3339(),
        last_update_time: Utc::now().to_rfc3339(),
    };

    let mut store = state.identity_store.write().await;
    if let Some(group) = store.groups.get_mut(&canonical_id) {
        group.aliases.push(alias.clone());
    }
    store.group_aliases.insert(alias_id.clone(), alias);

    Ok(VaultResponse::new().with_data(json!({ "id": alias_id, "canonical_id": canonical_id })))
}

pub async fn read_group_alias(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let store = state.identity_store.read().await;
    let alias = store.group_aliases.get(&id)
        .ok_or_else(|| VaultError::NotFound(format!("group alias {} not found", id)))?;
    Ok(VaultResponse::new().with_data(serde_json::to_value(alias).unwrap_or_default()))
}

pub async fn delete_group_alias(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let mut store = state.identity_store.write().await;
    store.group_aliases.remove(&id);
    Ok(VaultResponse::new())
}

pub async fn list_group_aliases(
    State(state): State<Arc<VaultState>>,
    headers: HeaderMap,
) -> Result<VaultResponse, VaultError> {
    let _token = extract_token(&headers)?;
    let store = state.identity_store.read().await;
    let keys: Vec<String> = store.group_aliases.keys().cloned().collect();
    Ok(VaultResponse::new().with_data(json!({ "keys": keys })))
}

pub fn router(state: Arc<VaultState>) -> Router {
    Router::new()
        // Entity
        .route("/v1/identity/entity", post(create_entity).get(list_entities))
        .route("/v1/identity/entity/id/:id", get(read_entity_by_id).post(update_entity).delete(delete_entity))
        .route("/v1/identity/entity/name/:name", get(read_entity_by_name))
        // Entity Alias
        .route("/v1/identity/entity-alias", post(create_entity_alias).get(list_entity_aliases))
        .route("/v1/identity/entity-alias/id/:id", get(read_entity_alias).delete(delete_entity_alias))
        // Group
        .route("/v1/identity/group", post(create_group).get(list_groups))
        .route("/v1/identity/group/id/:id", get(read_group_by_id).delete(delete_group))
        .route("/v1/identity/group/name/:name", get(read_group_by_name))
        // Group Alias
        .route("/v1/identity/group-alias", post(create_group_alias).get(list_group_aliases))
        .route("/v1/identity/group-alias/id/:id", get(read_group_alias).delete(delete_group_alias))
        .with_state(state)
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! HTTP route handlers for cave-chat.
//!
//! Endpoints
//! ---------
//!  GET  /api/chat/health
//!  GET  /api/chat/channels
//!  POST /api/chat/channels
//!  GET  /api/chat/channels/:id
//!  POST /api/chat/channels/:id/archive
//!  POST /api/chat/channels/:id/members
//!  DELETE /api/chat/channels/:id/members/:user_id
//!  GET  /api/chat/channels/:id/messages
//!  POST /api/chat/channels/:id/messages
//!  DELETE /api/chat/messages/:id
//!  GET  /api/chat/messages/:id/thread
//!  POST /api/chat/messages/:id/reactions
//!  DELETE /api/chat/messages/:id/reactions/:emoji/:user_id
//!  GET  /api/chat/search?q=…
//!  GET  /api/chat/presence/:user_id
//!  PUT  /api/chat/presence/:user_id
//!  GET  /api/chat/stats

use crate::models::{Channel, ChannelType, Message, MessageType, PresenceStatus, UserPresence};
use crate::store::ChatStore;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{delete, get, post},
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

// ── Shared state ──────────────────────────────────────────────────────────────

pub struct AppState {
    pub store: ChatStore,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            store: ChatStore::new(),
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

// ── Router ────────────────────────────────────────────────────────────────────

pub fn create_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/chat/health", get(health))
        // Channel endpoints
        .route("/api/chat/channels", get(list_channels).post(create_channel))
        .route("/api/chat/channels/{id}", get(get_channel))
        .route("/api/chat/channels/{id}/archive", post(archive_channel))
        .route("/api/chat/channels/{id}/members", post(add_member))
        .route("/api/chat/channels/{id}/members/{user_id}", delete(remove_member))
        .route(
            "/api/chat/channels/{id}/messages",
            get(get_channel_messages).post(post_message),
        )
        // Message endpoints
        .route("/api/chat/messages/{id}", delete(delete_message))
        .route("/api/chat/messages/{id}/thread", get(get_thread))
        .route("/api/chat/messages/{id}/reactions", post(add_reaction))
        .route(
            "/api/chat/messages/{id}/reactions/{emoji}/{user_id}",
            delete(remove_reaction),
        )
        // Search
        .route("/api/chat/search", get(search_messages))
        // Presence
        .route(
            "/api/chat/presence/{user_id}",
            get(get_presence).put(set_presence),
        )
        // Stats
        .route("/api/chat/stats", get(stats))
        .with_state(state)
}

// ── Handlers ──────────────────────────────────────────────────────────────────

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-chat",
        "status": "ok",
        "upstream": "LibreChat"
    }))
}

// -- Channels

#[derive(Debug, Deserialize)]
struct CreateChannelReq {
    name: String,
    channel_type: Option<String>,
}

async fn create_channel(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateChannelReq>,
) -> (StatusCode, Json<Channel>) {
    let ch_type = match req.channel_type.as_deref() {
        Some("private") => ChannelType::Private,
        Some("direct") => ChannelType::Direct,
        Some("announcement") => ChannelType::Announcement,
        _ => ChannelType::Public,
    };
    let ch = Channel::new(req.name, ch_type);
    let created = state.store.create_channel(ch);
    (StatusCode::CREATED, Json(created))
}

async fn list_channels(
    State(state): State<Arc<AppState>>,
) -> Json<Vec<Channel>> {
    Json(state.store.list_channels())
}

async fn get_channel(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<Channel>, StatusCode> {
    state
        .store
        .get_channel(&id)
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn archive_channel(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<Channel>, StatusCode> {
    state
        .store
        .archive_channel(&id)
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

#[derive(Debug, Deserialize)]
struct AddMemberReq {
    user_id: String,
}

async fn add_member(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<AddMemberReq>,
) -> Result<Json<Channel>, StatusCode> {
    state
        .store
        .add_member(&id, req.user_id)
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn remove_member(
    State(state): State<Arc<AppState>>,
    Path((id, user_id)): Path<(Uuid, String)>,
) -> Result<Json<Channel>, StatusCode> {
    state
        .store
        .remove_member(&id, &user_id)
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

// -- Messages

#[derive(Debug, Deserialize)]
struct PostMessageReq {
    author_id: String,
    content: String,
    message_type: Option<String>,
    thread_root_id: Option<Uuid>,
}

async fn post_message(
    State(state): State<Arc<AppState>>,
    Path(channel_id): Path<Uuid>,
    Json(req): Json<PostMessageReq>,
) -> (StatusCode, Json<Message>) {
    let msg_type = match req.message_type.as_deref() {
        Some("system") => MessageType::System,
        Some("file") => MessageType::File,
        Some("alert") => MessageType::Alert,
        _ => MessageType::Text,
    };
    let msg = Message {
        id: Uuid::new_v4(),
        channel_id,
        author_id: req.author_id,
        content: req.content,
        message_type: msg_type,
        created_at: Utc::now(),
        edited_at: None,
        reactions: vec![],
        thread_root_id: req.thread_root_id,
        thread_replies: vec![],
    };
    let created = state.store.create_message(msg);
    (StatusCode::CREATED, Json(created))
}

#[derive(Debug, Deserialize)]
struct GetMessagesQuery {
    limit: Option<usize>,
}

async fn get_channel_messages(
    State(state): State<Arc<AppState>>,
    Path(channel_id): Path<Uuid>,
    Query(q): Query<GetMessagesQuery>,
) -> Json<Vec<Message>> {
    let limit = q.limit.unwrap_or(50).min(200);
    Json(state.store.get_channel_messages(&channel_id, limit, None))
}

async fn delete_message(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<Message>, StatusCode> {
    state
        .store
        .delete_message(&id)
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn get_thread(
    State(state): State<Arc<AppState>>,
    Path(root_id): Path<Uuid>,
) -> Json<Vec<Message>> {
    Json(state.store.get_thread(&root_id))
}

// -- Reactions

#[derive(Debug, Deserialize)]
struct AddReactionReq {
    emoji: String,
    user_id: String,
}

async fn add_reaction(
    State(state): State<Arc<AppState>>,
    Path(message_id): Path<Uuid>,
    Json(req): Json<AddReactionReq>,
) -> Result<Json<Message>, StatusCode> {
    state
        .store
        .add_reaction(&message_id, req.emoji, req.user_id)
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn remove_reaction(
    State(state): State<Arc<AppState>>,
    Path((message_id, emoji, user_id)): Path<(Uuid, String, String)>,
) -> Result<Json<Message>, StatusCode> {
    state
        .store
        .remove_reaction(&message_id, &emoji, &user_id)
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

// -- Search

#[derive(Debug, Deserialize)]
struct SearchQuery {
    q: String,
    channel_id: Option<Uuid>,
}

async fn search_messages(
    State(state): State<Arc<AppState>>,
    Query(q): Query<SearchQuery>,
) -> Json<Vec<Message>> {
    Json(state.store.search_messages(&q.q, q.channel_id))
}

// -- Presence

#[derive(Debug, Deserialize, Serialize)]
struct SetPresenceReq {
    status: String,
}

async fn set_presence(
    State(state): State<Arc<AppState>>,
    Path(user_id): Path<String>,
    Json(req): Json<SetPresenceReq>,
) -> Json<UserPresence> {
    let status = match req.status.as_str() {
        "away" => PresenceStatus::Away,
        "dnd" | "do_not_disturb" => PresenceStatus::DoNotDisturb,
        "offline" => PresenceStatus::Offline,
        _ => PresenceStatus::Online,
    };
    let presence = UserPresence {
        user_id,
        status,
        last_seen: Utc::now(),
    };
    Json(state.store.set_presence(presence))
}

async fn get_presence(
    State(state): State<Arc<AppState>>,
    Path(user_id): Path<String>,
) -> Result<Json<UserPresence>, StatusCode> {
    state
        .store
        .get_presence(&user_id)
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

// -- Stats

async fn stats(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let s = state.store.compute_stats();
    Json(serde_json::json!({
        "total_channels": s.total_channels,
        "total_messages": s.total_messages,
        "active_users": s.active_users,
        "messages_today": s.messages_today,
    }))
}

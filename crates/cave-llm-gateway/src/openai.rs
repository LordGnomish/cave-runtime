// SPDX-License-Identifier: AGPL-3.0-or-later
//! OpenAI API compatibility types.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── Chat Completions ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub top_p: Option<f32>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub stream: Option<bool>,
    #[serde(default)]
    pub stop: Option<StopSequence>,
    #[serde(default)]
    pub presence_penalty: Option<f32>,
    #[serde(default)]
    pub frequency_penalty: Option<f32>,
    #[serde(default)]
    pub n: Option<u32>,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub tools: Option<Vec<Tool>>,
    #[serde(default)]
    pub tool_choice: Option<serde_json::Value>,
    #[serde(default)]
    pub response_format: Option<ResponseFormat>,
    #[serde(default)]
    pub seed: Option<i64>,
    #[serde(default)]
    pub logprobs: Option<bool>,
}

impl ChatCompletionRequest {
    pub fn is_streaming(&self) -> bool {
        self.stream.unwrap_or(false)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StopSequence {
    Single(String),
    Multiple(Vec<String>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: Role,
    pub content: MessageContent,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self { role: Role::System, content: MessageContent::Text(content.into()), name: None, tool_calls: None, tool_call_id: None }
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self { role: Role::User, content: MessageContent::Text(content.into()), name: None, tool_calls: None, tool_call_id: None }
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self { role: Role::Assistant, content: MessageContent::Text(content.into()), name: None, tool_calls: None, tool_call_id: None }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
    Function,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Parts(Vec<ContentPart>),
}

impl MessageContent {
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(s) => Some(s),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentPart {
    #[serde(rename = "type")]
    pub part_type: String,
    pub text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tool {
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: FunctionDef,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionDef {
    pub name: String,
    pub description: Option<String>,
    pub parameters: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub call_type: String,
    pub function: FunctionCall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseFormat {
    #[serde(rename = "type")]
    pub format_type: String,
}

// ── Response ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionResponse {
    pub id: String,
    pub object: String,
    pub created: i64,
    pub model: String,
    pub choices: Vec<Choice>,
    pub usage: Usage,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_fingerprint: Option<String>,
}

impl ChatCompletionResponse {
    pub fn simple(model: &str, content: String, usage: Usage) -> Self {
        Self {
            id: format!("chatcmpl-{}", uuid::Uuid::new_v4().to_string().replace('-', "")),
            object: "chat.completion".into(),
            created: chrono::Utc::now().timestamp(),
            model: model.to_string(),
            choices: vec![Choice {
                index: 0,
                message: Some(ChatMessage::assistant(content)),
                delta: None,
                finish_reason: Some("stop".into()),
                logprobs: None,
            }],
            usage,
            system_fingerprint: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Choice {
    pub index: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delta: Option<DeltaMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logprobs: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeltaMessage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<Role>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

impl Usage {
    pub fn new(prompt: u32, completion: u32) -> Self {
        Self { prompt_tokens: prompt, completion_tokens: completion, total_tokens: prompt + completion }
    }
}

// ── SSE streaming chunk ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct ChatCompletionChunk {
    pub id: String,
    pub object: String,
    pub created: i64,
    pub model: String,
    pub choices: Vec<Choice>,
}

impl ChatCompletionChunk {
    pub fn content_delta(id: &str, model: &str, content: &str) -> Self {
        Self {
            id: id.to_string(),
            object: "chat.completion.chunk".into(),
            created: chrono::Utc::now().timestamp(),
            model: model.to_string(),
            choices: vec![Choice {
                index: 0,
                message: None,
                delta: Some(DeltaMessage { role: None, content: Some(content.to_string()), tool_calls: None }),
                finish_reason: None,
                logprobs: None,
            }],
        }
    }

    pub fn stop(id: &str, model: &str) -> Self {
        Self {
            id: id.to_string(),
            object: "chat.completion.chunk".into(),
            created: chrono::Utc::now().timestamp(),
            model: model.to_string(),
            choices: vec![Choice {
                index: 0,
                message: None,
                delta: Some(DeltaMessage { role: None, content: None, tool_calls: None }),
                finish_reason: Some("stop".into()),
                logprobs: None,
            }],
        }
    }
}

// ── Models list ───────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct ModelObject {
    pub id: String,
    pub object: String,
    pub created: i64,
    pub owned_by: String,
}

#[derive(Debug, Serialize)]
pub struct ModelList {
    pub object: String,
    pub data: Vec<ModelObject>,
}

// ── Error response ────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct OpenAIError {
    pub error: OpenAIErrorBody,
}

#[derive(Debug, Serialize)]
pub struct OpenAIErrorBody {
    pub message: String,
    #[serde(rename = "type")]
    pub error_type: String,
    pub param: Option<String>,
    pub code: Option<String>,
}

impl OpenAIError {
    pub fn invalid_request(message: &str) -> Self {
        Self { error: OpenAIErrorBody { message: message.to_string(), error_type: "invalid_request_error".into(), param: None, code: None } }
    }

    pub fn server_error(message: &str) -> Self {
        Self { error: OpenAIErrorBody { message: message.to_string(), error_type: "server_error".into(), param: None, code: Some("internal_error".into()) } }
    }

    pub fn rate_limit(message: &str) -> Self {
        Self { error: OpenAIErrorBody { message: message.to_string(), error_type: "requests".into(), param: None, code: Some("rate_limit_exceeded".into()) } }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_message_construction() {
        let msg = ChatMessage::user("Hello, how are you?");
        assert_eq!(msg.role, Role::User);
        assert_eq!(msg.content.as_text(), Some("Hello, how are you?"));
    }

    #[test]
    fn completion_response_construction() {
        let usage = Usage::new(10, 20);
        let resp = ChatCompletionResponse::simple("gpt-4", "I'm doing well!".into(), usage);
        assert_eq!(resp.object, "chat.completion");
        assert_eq!(resp.choices.len(), 1);
        assert_eq!(resp.usage.total_tokens, 30);
    }

    #[test]
    fn streaming_chunk_construction() {
        let chunk = ChatCompletionChunk::content_delta("chatcmpl-123", "gpt-4", "Hello");
        assert_eq!(chunk.object, "chat.completion.chunk");
        assert!(chunk.choices[0].delta.as_ref().unwrap().content.is_some());
    }
}

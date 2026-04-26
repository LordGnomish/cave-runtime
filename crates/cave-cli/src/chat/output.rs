//! Chat message types + pipe-mode (script-friendly) output formatting.
//!
//! `cavectl chat --pipe` writes assistant tokens to stdout as they arrive,
//! suppressing TUI chrome. Mode rendering: PlainText (default), JsonLines.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatRole {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
    pub tenant_id: String,
}

impl ChatMessage {
    pub fn user(tenant_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::User,
            content: content.into(),
            tenant_id: tenant_id.into(),
        }
    }

    pub fn assistant(tenant_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::Assistant,
            content: content.into(),
            tenant_id: tenant_id.into(),
        }
    }

    pub fn system(tenant_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::System,
            content: content.into(),
            tenant_id: tenant_id.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamChunk {
    pub conversation_id: String,
    pub delta: String,
    pub finish: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipeFormat {
    PlainText,
    JsonLines,
}

impl PipeFormat {
    /// Render a stream chunk for `--pipe` consumers.
    /// PlainText: emit only the delta, no trailing newline (caller adds on `finish`).
    /// JsonLines: emit `{conversation_id, delta, finish}` per chunk.
    pub fn render_chunk(self, c: &StreamChunk) -> String {
        match self {
            PipeFormat::PlainText => c.delta.clone(),
            PipeFormat::JsonLines => serde_json::to_string(c).unwrap_or_default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// cite: chat output — user constructor sets role
    #[test]
    fn chat_acme_user_constructor_sets_role() {
        let tenant_id = "acme";
        let m = ChatMessage::user(tenant_id, "hi");
        assert_eq!(m.role, ChatRole::User);
        assert_eq!(m.tenant_id, tenant_id);
    }

    /// cite: chat output — assistant constructor sets role
    #[test]
    fn chat_globex_assistant_constructor_sets_role() {
        let tenant_id = "globex";
        let m = ChatMessage::assistant(tenant_id, "ok");
        assert_eq!(m.role, ChatRole::Assistant);
    }

    /// cite: chat output — system constructor sets role
    #[test]
    fn chat_initech_system_constructor_sets_role() {
        let tenant_id = "initech";
        let m = ChatMessage::system(tenant_id, "you are ...");
        assert_eq!(m.role, ChatRole::System);
    }

    /// cite: pipe format — PlainText emits only delta
    #[test]
    fn pipe_acme_plaintext_emits_delta_only() {
        let _tenant_id = "acme";
        let c = StreamChunk {
            conversation_id: "c1".into(),
            delta: "hello".into(),
            finish: false,
        };
        assert_eq!(PipeFormat::PlainText.render_chunk(&c), "hello");
    }

    /// cite: pipe format — JsonLines emits structured object
    #[test]
    fn pipe_globex_jsonlines_emits_structured() {
        let _tenant_id = "globex";
        let c = StreamChunk {
            conversation_id: "c1".into(),
            delta: "hello".into(),
            finish: true,
        };
        let out = PipeFormat::JsonLines.render_chunk(&c);
        let parsed: StreamChunk = serde_json::from_str(&out).unwrap();
        assert_eq!(parsed, c);
    }
}

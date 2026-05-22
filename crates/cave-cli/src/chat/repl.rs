// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Headless REPL state machine driving `cavectl chat` (interactive ratatui mode).
//!
//! The binary translates `crossterm::event::KeyEvent` into `ReplCommand` and
//! feeds them here; this module owns no terminal handle. Output: `ReplEffect`
//! describing what the renderer should do next (append text, send to backend,
//! exit, switch conversation, etc.).

use serde::{Deserialize, Serialize};

use super::output::{ChatMessage, StreamChunk};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplCommand {
    /// User typed a printable character.
    Char(char),
    /// Backspace key.
    Backspace,
    /// Enter — submit current buffer (if non-empty and not a slash command).
    Submit,
    /// Streaming chunk arrived from backend.
    StreamChunk(StreamChunk),
    /// User invoked a slash command (e.g. `/quit`, `/clear`, `/tools`).
    Slash(String),
    /// External signal: cancel the in-flight request.
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplEffect {
    /// No-op — only state changed (cursor moved, buffer updated).
    None,
    /// Renderer should display this assistant delta inline.
    Render { delta: String },
    /// Submit a fully composed user message to backend.
    Send(ChatMessage),
    /// Cancel any pending in-flight request.
    CancelInFlight,
    /// Clear the visible transcript (and message history).
    Clear,
    /// Print a help / info banner to the renderer.
    Banner(String),
    /// Exit the REPL with the given exit code.
    Exit(i32),
}

#[derive(Debug, Clone)]
pub struct ReplState {
    pub tenant_id: String,
    pub conversation_id: String,
    pub buffer: String,
    pub messages: Vec<ChatMessage>,
    pub streaming: bool,
}

impl ReplState {
    pub fn new(tenant_id: impl Into<String>, conversation_id: impl Into<String>) -> Self {
        Self {
            tenant_id: tenant_id.into(),
            conversation_id: conversation_id.into(),
            buffer: String::new(),
            messages: vec![],
            streaming: false,
        }
    }

    pub fn handle(&mut self, cmd: ReplCommand) -> ReplEffect {
        match cmd {
            ReplCommand::Char(c) => {
                self.buffer.push(c);
                ReplEffect::None
            }
            ReplCommand::Backspace => {
                self.buffer.pop();
                ReplEffect::None
            }
            ReplCommand::Submit => {
                let trimmed = self.buffer.trim();
                if trimmed.is_empty() {
                    return ReplEffect::None;
                }
                if let Some(rest) = trimmed.strip_prefix('/') {
                    let cmd_str = rest.to_string();
                    self.buffer.clear();
                    return self.handle(ReplCommand::Slash(cmd_str));
                }
                let msg = ChatMessage::user(&self.tenant_id, trimmed);
                self.messages.push(msg.clone());
                self.buffer.clear();
                self.streaming = true;
                ReplEffect::Send(msg)
            }
            ReplCommand::StreamChunk(c) => {
                if c.conversation_id != self.conversation_id {
                    return ReplEffect::None;
                }
                if let Some(last) = self
                    .messages
                    .last_mut()
                    .filter(|m| m.role == super::output::ChatRole::Assistant)
                {
                    last.content.push_str(&c.delta);
                } else {
                    self.messages
                        .push(ChatMessage::assistant(&self.tenant_id, c.delta.clone()));
                }
                if c.finish {
                    self.streaming = false;
                }
                ReplEffect::Render { delta: c.delta }
            }
            ReplCommand::Slash(s) => match s.as_str() {
                "quit" | "exit" | "q" => ReplEffect::Exit(0),
                "clear" | "cls" => {
                    self.messages.clear();
                    ReplEffect::Clear
                }
                "help" | "?" => {
                    ReplEffect::Banner("/quit | /clear | /help | /tools | /conv".to_string())
                }
                "tools" => ReplEffect::Banner("(tools list shown by binary)".to_string()),
                "conv" => ReplEffect::Banner(format!(
                    "conversation_id={} tenant_id={}",
                    self.conversation_id, self.tenant_id
                )),
                other => ReplEffect::Banner(format!("unknown slash command: /{other}")),
            },
            ReplCommand::Cancel => {
                if self.streaming {
                    self.streaming = false;
                    ReplEffect::CancelInFlight
                } else {
                    ReplEffect::None
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReplMode {
    Interactive,
    Pipe,
    ToolCalling,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::output::ChatRole;

    fn state() -> ReplState {
        ReplState::new("acme", "conv-1")
    }

    /// cite: REPL state — buffer accumulates printable chars
    #[test]
    fn repl_acme_char_appends_to_buffer() {
        let _tenant_id = "acme";
        let mut s = state();
        assert_eq!(s.handle(ReplCommand::Char('h')), ReplEffect::None);
        s.handle(ReplCommand::Char('i'));
        assert_eq!(s.buffer, "hi");
    }

    /// cite: REPL state — backspace shrinks buffer
    #[test]
    fn repl_globex_backspace_shrinks_buffer() {
        let _tenant_id = "globex";
        let mut s = ReplState::new("globex", "conv-1");
        s.buffer = "hello".to_string();
        s.handle(ReplCommand::Backspace);
        assert_eq!(s.buffer, "hell");
    }

    /// cite: REPL state — backspace on empty buffer is no-op
    #[test]
    fn repl_initech_backspace_empty_is_noop() {
        let _tenant_id = "initech";
        let mut s = ReplState::new("initech", "conv-1");
        assert_eq!(s.handle(ReplCommand::Backspace), ReplEffect::None);
        assert_eq!(s.buffer, "");
    }

    /// cite: REPL state — submit emits Send and clears buffer
    #[test]
    fn repl_acme_submit_emits_send_and_clears_buffer() {
        let _tenant_id = "acme";
        let mut s = state();
        s.buffer = "hello".to_string();
        let eff = s.handle(ReplCommand::Submit);
        match eff {
            ReplEffect::Send(m) => {
                assert_eq!(m.content, "hello");
                assert_eq!(m.role, ChatRole::User);
                assert_eq!(m.tenant_id, "acme");
            }
            other => panic!("expected Send, got {other:?}"),
        }
        assert!(s.buffer.is_empty());
        assert!(s.streaming);
    }

    /// cite: REPL state — submit on empty buffer is no-op
    #[test]
    fn repl_globex_submit_empty_is_noop() {
        let _tenant_id = "globex";
        let mut s = ReplState::new("globex", "conv-1");
        assert_eq!(s.handle(ReplCommand::Submit), ReplEffect::None);
    }

    /// cite: REPL state — slash command `/quit` exits with 0
    #[test]
    fn repl_acme_slash_quit_exits_zero() {
        let _tenant_id = "acme";
        let mut s = state();
        s.buffer = "/quit".to_string();
        assert_eq!(s.handle(ReplCommand::Submit), ReplEffect::Exit(0));
    }

    /// cite: REPL state — slash `/clear` resets messages and emits Clear
    #[test]
    fn repl_acme_slash_clear_resets_history() {
        let _tenant_id = "acme";
        let mut s = state();
        s.messages.push(ChatMessage::user("acme", "hi"));
        s.buffer = "/clear".to_string();
        assert_eq!(s.handle(ReplCommand::Submit), ReplEffect::Clear);
        assert!(s.messages.is_empty());
    }

    /// cite: REPL state — `/help` returns a Banner
    #[test]
    fn repl_initech_slash_help_returns_banner() {
        let _tenant_id = "initech";
        let mut s = ReplState::new("initech", "conv-1");
        s.buffer = "/help".to_string();
        match s.handle(ReplCommand::Submit) {
            ReplEffect::Banner(b) => assert!(b.contains("/quit")),
            other => panic!("expected Banner, got {other:?}"),
        }
    }

    /// cite: REPL state — `/conv` includes conversation_id and tenant_id
    #[test]
    fn repl_acme_slash_conv_includes_ids() {
        let _tenant_id = "acme";
        let mut s = state();
        s.buffer = "/conv".to_string();
        match s.handle(ReplCommand::Submit) {
            ReplEffect::Banner(b) => {
                assert!(b.contains("conv-1"));
                assert!(b.contains("acme"));
            }
            other => panic!("expected Banner, got {other:?}"),
        }
    }

    /// cite: REPL state — unknown slash is a Banner not Exit
    #[test]
    fn repl_globex_unknown_slash_is_banner() {
        let _tenant_id = "globex";
        let mut s = ReplState::new("globex", "conv-1");
        s.buffer = "/zzz".to_string();
        match s.handle(ReplCommand::Submit) {
            ReplEffect::Banner(b) => assert!(b.contains("unknown")),
            other => panic!("expected Banner, got {other:?}"),
        }
    }

    /// cite: REPL state — chunks for matching conversation append to assistant
    #[test]
    fn repl_acme_stream_chunk_appends_to_assistant() {
        let _tenant_id = "acme";
        let mut s = state();
        s.handle(ReplCommand::StreamChunk(StreamChunk {
            conversation_id: "conv-1".into(),
            delta: "hel".into(),
            finish: false,
        }));
        s.handle(ReplCommand::StreamChunk(StreamChunk {
            conversation_id: "conv-1".into(),
            delta: "lo".into(),
            finish: true,
        }));
        assert_eq!(s.messages.len(), 1);
        assert_eq!(s.messages[0].content, "hello");
        assert!(!s.streaming);
    }

    /// cite: REPL state — chunk for foreign conversation ignored
    #[test]
    fn repl_globex_stream_chunk_foreign_conv_ignored() {
        let _tenant_id = "globex";
        let mut s = ReplState::new("globex", "conv-1");
        s.handle(ReplCommand::StreamChunk(StreamChunk {
            conversation_id: "other-conv".into(),
            delta: "hi".into(),
            finish: true,
        }));
        assert!(s.messages.is_empty());
    }

    /// cite: REPL state — Cancel during streaming emits CancelInFlight
    #[test]
    fn repl_acme_cancel_during_streaming() {
        let _tenant_id = "acme";
        let mut s = state();
        s.streaming = true;
        assert_eq!(s.handle(ReplCommand::Cancel), ReplEffect::CancelInFlight);
        assert!(!s.streaming);
    }

    /// cite: REPL state — Cancel idle is no-op
    #[test]
    fn repl_initech_cancel_idle_is_noop() {
        let _tenant_id = "initech";
        let mut s = ReplState::new("initech", "conv-1");
        assert_eq!(s.handle(ReplCommand::Cancel), ReplEffect::None);
    }
}

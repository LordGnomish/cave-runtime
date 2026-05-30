// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Conversation export to plain text / markdown.
//!
//! Faithful line-port of danny-avila/LibreChat v0.7.6
//! `client/src/hooks/Conversations/useExportConversation.ts` — the pure
//! string-formatting logic behind `exportMarkdown`, `exportText`, and the shared
//! `getMessageText` / `formatText` helpers.
//!
//! The upstream hook also wires browser download, screenshot capture, CSV and
//! JSON serialisation, and the async `buildMessageTree` walk; those belong to the
//! UI / persistence layers and are intentionally NOT ported here. Only the
//! in-process header + per-message formatting transformation lives in-crate.

use serde::{Deserialize, Serialize};

/// Export render format. Mirrors the `format` argument of upstream `getMessageText`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExportFormat {
    /// Plain text: `>> sender:\n{text}`.
    Text,
    /// Markdown: `**sender**\n{text}`.
    Markdown,
}

/// Conversation header metadata included at the top of an export.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationMeta {
    pub conversation_id: String,
    pub endpoint: String,
    pub title: String,
}

/// A single message to render in an export.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportMessage {
    pub sender: String,
    pub text: String,
    pub error: bool,
    pub unfinished: bool,
}

/// Format one sender+text pair.
///
/// Faithful port of upstream `formatText` (useExportConversation.ts L62-67):
/// text format -> `>> ${sender}:\n${text}`, markdown -> `**${sender}**\n${text}`.
pub fn format_message_text(sender: &str, text: &str, format: ExportFormat) -> String {
    match format {
        ExportFormat::Text => format!(">> {sender}:\n{text}"),
        ExportFormat::Markdown => format!("**{sender}**\n{text}"),
    }
}

/// Render a full conversation as markdown.
///
/// Faithful port of upstream `exportMarkdown` (useExportConversation.ts L219-256):
/// a `# Conversation` header block, then a `## History` section with each message
/// rendered via [`format_message_text`] and annotated for error / unfinished state.
/// The volatile `- exportAt:` timestamp line is omitted so output is deterministic.
pub fn export_markdown(meta: &ConversationMeta, messages: &[ExportMessage]) -> String {
    let mut data = String::new();
    data.push_str("# Conversation\n");
    data.push_str(&format!("- conversationId: {}\n", meta.conversation_id));
    data.push_str(&format!("- endpoint: {}\n", meta.endpoint));
    data.push_str(&format!("- title: {}\n", meta.title));

    data.push_str("\n## History\n");
    for message in messages {
        data.push_str(&format_message_text(
            &message.sender,
            &message.text,
            ExportFormat::Markdown,
        ));
        data.push('\n');
        if message.error {
            data.push_str("*(This is an error message)*\n");
        }
        if message.unfinished {
            data.push_str("*(This is an unfinished message)*\n");
        }
        data.push_str("\n\n");
    }
    data
}

/// Render a full conversation as plain text.
///
/// Faithful port of upstream `exportText` (useExportConversation.ts L274-313):
/// a `Conversation` header block delimited by `########################`, then a
/// `History` section with each message rendered via [`format_message_text`] and
/// annotated for error / unfinished state. The volatile `exportAt:` timestamp line
/// is omitted so output is deterministic.
pub fn export_text(meta: &ConversationMeta, messages: &[ExportMessage]) -> String {
    let mut data = String::new();
    data.push_str("Conversation\n");
    data.push_str("########################\n");
    data.push_str(&format!("conversationId: {}\n", meta.conversation_id));
    data.push_str(&format!("endpoint: {}\n", meta.endpoint));
    data.push_str(&format!("title: {}\n", meta.title));

    data.push_str("\nHistory\n########################\n");
    for message in messages {
        data.push_str(&format_message_text(
            &message.sender,
            &message.text,
            ExportFormat::Text,
        ));
        data.push('\n');
        if message.error {
            data.push_str("(This is an error message)\n");
        }
        if message.unfinished {
            data.push_str("(This is an unfinished message)\n");
        }
        data.push_str("\n\n");
    }
    data
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_roundtrip_text_and_md() {
        assert_eq!(format_message_text("A", "b", ExportFormat::Text), ">> A:\nb");
        assert_eq!(
            format_message_text("A", "b", ExportFormat::Markdown),
            "**A**\nb"
        );
    }

    #[test]
    fn markdown_empty_messages_still_has_history_header() {
        let meta = ConversationMeta {
            conversation_id: "c".into(),
            endpoint: "e".into(),
            title: "t".into(),
        };
        let out = export_markdown(&meta, &[]);
        assert!(out.contains("\n## History\n"));
    }
}

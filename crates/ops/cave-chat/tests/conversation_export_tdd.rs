// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict-TDD integration test for conversation export to text / markdown.
//!
//! Faithful port of danny-avila/LibreChat v0.7.6
//! `client/src/hooks/Conversations/useExportConversation.ts`:
//!   - getMessageText / formatText
//!   - exportMarkdown body (header + "## History" + per-message md format)
//!   - exportText body (header + "History" + per-message text format)

use cave_chat::conversation_export::{
    export_markdown, export_text, format_message_text, ExportFormat, ExportMessage,
    ConversationMeta,
};

fn meta() -> ConversationMeta {
    ConversationMeta {
        conversation_id: "conv-1".into(),
        endpoint: "openAI".into(),
        title: "Greetings".into(),
    }
}

fn msgs() -> Vec<ExportMessage> {
    vec![
        ExportMessage {
            sender: "User".into(),
            text: "Hello there".into(),
            error: false,
            unfinished: false,
        },
        ExportMessage {
            sender: "Assistant".into(),
            text: "Hi!".into(),
            error: false,
            unfinished: false,
        },
    ]
}

#[test]
fn format_text_uses_quote_prefix_for_text() {
    let out = format_message_text("User", "Hello", ExportFormat::Text);
    assert_eq!(out, ">> User:\nHello");
}

#[test]
fn format_text_uses_bold_for_markdown() {
    let out = format_message_text("User", "Hello", ExportFormat::Markdown);
    assert_eq!(out, "**User**\nHello");
}

#[test]
fn markdown_export_has_header_and_history_section() {
    let out = export_markdown(&meta(), &msgs());
    assert!(out.starts_with("# Conversation\n"), "got: {out}");
    assert!(out.contains("- conversationId: conv-1\n"));
    assert!(out.contains("- endpoint: openAI\n"));
    assert!(out.contains("- title: Greetings\n"));
    assert!(out.contains("\n## History\n"));
    assert!(out.contains("**User**\nHello there\n"));
    assert!(out.contains("**Assistant**\nHi!\n"));
}

#[test]
fn text_export_has_header_and_history_section() {
    let out = export_text(&meta(), &msgs());
    assert!(out.starts_with("Conversation\n########################\n"), "got: {out}");
    assert!(out.contains("conversationId: conv-1\n"));
    assert!(out.contains("\nHistory\n########################\n"));
    assert!(out.contains(">> User:\nHello there\n"));
    assert!(out.contains(">> Assistant:\nHi!\n"));
}

#[test]
fn markdown_export_annotates_error_and_unfinished() {
    let messages = vec![
        ExportMessage {
            sender: "Assistant".into(),
            text: "boom".into(),
            error: true,
            unfinished: false,
        },
        ExportMessage {
            sender: "Assistant".into(),
            text: "partial".into(),
            error: false,
            unfinished: true,
        },
    ];
    let out = export_markdown(&meta(), &messages);
    assert!(out.contains("*(This is an error message)*\n"));
    assert!(out.contains("*(This is an unfinished message)*\n"));
}

#[test]
fn text_export_annotates_error_and_unfinished() {
    let messages = vec![ExportMessage {
        sender: "Assistant".into(),
        text: "boom".into(),
        error: true,
        unfinished: false,
    }];
    let out = export_text(&meta(), &messages);
    assert!(out.contains("(This is an error message)\n"));
    assert!(!out.contains("*(This is an error message)*"));
}

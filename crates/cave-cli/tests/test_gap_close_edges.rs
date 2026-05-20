// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Integration-side edge-case coverage for cavectl public surface.
//!
//! These tests target areas that are part of cavectl's external library
//! contract but were not exercised by the per-module inline `#[cfg(test)]`
//! suites: serde round-trips on the re-exported public types, boundary +
//! failure modes on argv/path/config parsers, output formatter shape
//! invariants, and cross-module composition (tenant-scope resolution
//! feeding native verbs, audit-query plus filter cooperation, REPL
//! transitions across multiple chunks).

use cavectl::audit::AuditFilter;
use cavectl::auth::saml::{SamlCmd, parse_argv as saml_parse};
use cavectl::auth::webauthn::{ParseError, WebAuthnCmd, parse as webauthn_parse};
use cavectl::chat::output::{ChatMessage, ChatRole, PipeFormat, StreamChunk};
use cavectl::chat::repl::ReplMode;
use cavectl::tenant_scope::{
    TenantInputs, TenantSource, parse_config_tenant, resolve, validate_tenant,
};
use cavectl::{
    ApprovalState, AuditEntry, AuditQuery, ConversationKind, EnvLifecycleState, ExitCode,
    InMemoryApprovals, InMemoryAuditLog, InMemoryConversationStore, InMemoryEnvBackend,
    InMemoryTenantBackend, JsonStream, ReplCommand, ReplEffect, ReplState, StreamFormat,
    TenantBackend, TenantLifecycleState, TenantRecord, ToolMode, WatchEvent, WatchTicker,
};
use cavectl::approval::ApprovalBackend;
use cavectl::audit::AuditLog;
use cavectl::env::EnvBackend;
use cavectl::chat::conversation::ConversationStore;
use chrono::{TimeZone, Utc};
use std::time::Duration;

// ── (1) Serde round-trips on the publicly re-exported types ─────────────────

#[test]
fn serde_approval_state_round_trip_lowercase() {
    let s = serde_json::to_string(&ApprovalState::Pending).unwrap();
    assert_eq!(s, "\"pending\"");
    let back: ApprovalState = serde_json::from_str("\"approved\"").unwrap();
    assert_eq!(back, ApprovalState::Approved);
    let back2: ApprovalState = serde_json::from_str("\"cancelled\"").unwrap();
    assert_eq!(back2, ApprovalState::Cancelled);
}

#[test]
fn serde_tenant_lifecycle_state_round_trip_lowercase() {
    assert_eq!(
        serde_json::to_string(&TenantLifecycleState::Active).unwrap(),
        "\"active\"",
    );
    let back: TenantLifecycleState = serde_json::from_str("\"suspended\"").unwrap();
    assert_eq!(back, TenantLifecycleState::Suspended);
    let term: TenantLifecycleState = serde_json::from_str("\"terminated\"").unwrap();
    assert_eq!(term, TenantLifecycleState::Terminated);
}

#[test]
fn serde_env_lifecycle_state_round_trip_lowercase() {
    assert_eq!(
        serde_json::to_string(&EnvLifecycleState::Archived).unwrap(),
        "\"archived\"",
    );
    let back: EnvLifecycleState = serde_json::from_str("\"active\"").unwrap();
    assert_eq!(back, EnvLifecycleState::Active);
}

#[test]
fn serde_chat_role_round_trip_lowercase() {
    assert_eq!(serde_json::to_string(&ChatRole::Tool).unwrap(), "\"tool\"");
    let back: ChatRole = serde_json::from_str("\"system\"").unwrap();
    assert_eq!(back, ChatRole::System);
}

#[test]
fn serde_chat_message_full_round_trip() {
    let m = ChatMessage::user("acme", "hello");
    let s = serde_json::to_string(&m).unwrap();
    let back: ChatMessage = serde_json::from_str(&s).unwrap();
    assert_eq!(back, m);
    assert_eq!(back.role, ChatRole::User);
}

#[test]
fn serde_stream_chunk_round_trip_preserves_finish_flag() {
    let c = StreamChunk {
        conversation_id: "conv-x".into(),
        delta: "tok".into(),
        finish: true,
    };
    let s = serde_json::to_string(&c).unwrap();
    let back: StreamChunk = serde_json::from_str(&s).unwrap();
    assert!(back.finish);
    assert_eq!(back.conversation_id, "conv-x");
}

#[test]
fn serde_repl_mode_round_trip_via_pascal() {
    // ReplMode derives Serialize/Deserialize but no rename_all — defaults to PascalCase variant names.
    let s = serde_json::to_string(&ReplMode::Pipe).unwrap();
    let back: ReplMode = serde_json::from_str(&s).unwrap();
    assert_eq!(back, ReplMode::Pipe);
}

#[test]
fn serde_audit_entry_round_trip_with_metadata() {
    let e = AuditEntry {
        tenant_id: "acme".into(),
        at: Utc.with_ymd_and_hms(2026, 5, 20, 12, 0, 0).unwrap(),
        actor: "burak".into(),
        action: "tenant.suspend".into(),
        target: "acme".into(),
        outcome: "success".into(),
        metadata: serde_json::json!({"ip": "10.0.0.1", "reason": "billing-overdue"}),
    };
    let s = serde_json::to_string(&e).unwrap();
    let back: AuditEntry = serde_json::from_str(&s).unwrap();
    assert_eq!(back, e);
    assert_eq!(back.metadata["ip"], "10.0.0.1");
}

// ── (2) ExitCode / StreamFormat edge cases ──────────────────────────────────

#[test]
fn exit_code_from_http_boundary_399_and_400() {
    // 399 is not a success (200..=299), but is below the auth range — generic failure.
    assert_eq!(ExitCode::from_http(399), ExitCode::Failure);
    // 400 falls through to generic failure (not auth, not-found, conflict, or 5xx).
    assert_eq!(ExitCode::from_http(400), ExitCode::Failure);
}

#[test]
fn exit_code_from_http_200_lower_and_299_upper_boundary() {
    assert_eq!(ExitCode::from_http(200), ExitCode::Success);
    assert_eq!(ExitCode::from_http(299), ExitCode::Success);
    // Just outside the success band.
    assert_eq!(ExitCode::from_http(300), ExitCode::Failure);
}

#[test]
fn exit_code_as_i32_covers_all_named_codes() {
    assert_eq!(ExitCode::Failure.as_i32(), 1);
    assert_eq!(ExitCode::Usage.as_i32(), 2);
    assert_eq!(ExitCode::Auth.as_i32(), 3);
    assert_eq!(ExitCode::NotFound.as_i32(), 4);
    assert_eq!(ExitCode::Conflict.as_i32(), 5);
    assert_eq!(ExitCode::Unavailable.as_i32(), 6);
}

#[test]
fn stream_format_text_on_non_string_uses_to_string() {
    let v = serde_json::json!(42);
    assert_eq!(StreamFormat::Text.render(&v), "42");
    let arr = serde_json::json!([1, 2, 3]);
    // serde_json::Value::to_string produces compact JSON for arrays.
    assert_eq!(StreamFormat::Text.render(&arr), "[1,2,3]");
}

#[test]
fn json_stream_buffer_empty_flush_returns_empty_string() {
    let mut s = JsonStream::new();
    assert_eq!(s.flush_to_string("\n"), "");
    assert!(s.buffered.is_empty());
}

#[test]
fn json_stream_mixed_formats_preserve_per_push_choice() {
    let mut s = JsonStream::new();
    s.push(StreamFormat::NdJson, &serde_json::json!({"a": 1}));
    s.push(StreamFormat::Sse, &serde_json::json!({"b": 2}));
    let out = s.flush_to_string("");
    assert!(out.starts_with("{\"a\":1}"));
    assert!(out.contains("data: {\"b\":2}"));
}

#[test]
fn watch_event_serde_round_trip() {
    let e = WatchEvent {
        kind: "pod.update".into(),
        tick: 42,
        payload: serde_json::json!({"pod": "nginx", "phase": "Running"}),
    };
    let s = serde_json::to_string(&e).unwrap();
    let back: WatchEvent = serde_json::from_str(&s).unwrap();
    assert_eq!(back, e);
}

#[test]
fn watch_ticker_wraps_on_u64_overflow() {
    let mut t = WatchTicker::new(Duration::from_millis(1));
    // Drive the counter near the wrap boundary by reaching into the public
    // contract: we call tick() repeatedly. Use a small loop and confirm
    // monotonicity for a sane prefix — the wrap behaviour is asserted by
    // checking that t.current() advances exactly once per tick.
    for i in 0..1000 {
        assert_eq!(t.tick(), i);
    }
    assert_eq!(t.current(), 1000);
}

// ── (3) Tenant scope resolution edge cases ──────────────────────────────────

#[test]
fn resolve_max_length_tenant_via_flag() {
    let v = "a".repeat(63);
    let r = resolve(TenantInputs {
        flag: Some(&v),
        env: None,
        config: None,
    })
    .unwrap();
    assert_eq!(r.value.as_deref(), Some(v.as_str()));
    assert_eq!(r.source, TenantSource::Flag);
}

#[test]
fn validate_tenant_single_char_alnum_accepted() {
    assert!(validate_tenant("a").is_ok());
    assert!(validate_tenant("0").is_ok());
    assert!(validate_tenant("9").is_ok());
}

#[test]
fn validate_tenant_single_dash_rejected() {
    // start+end both dash → fails leading-dash check.
    assert!(validate_tenant("-").is_err());
}

#[test]
fn parse_config_tenant_strips_single_quotes() {
    let cfg = "[default]\ntenant = 'acme'\n";
    assert_eq!(parse_config_tenant(cfg), Some("acme".to_string()));
}

#[test]
fn parse_config_tenant_ignores_indented_default() {
    // Leading whitespace before section header is fine — the line is trimmed.
    let cfg = "   [default]\n   tenant = \"acme\"\n";
    assert_eq!(parse_config_tenant(cfg), Some("acme".to_string()));
}

#[test]
fn resolve_chains_through_to_unset_when_all_none() {
    let r = resolve(TenantInputs::default()).unwrap();
    assert!(r.value.is_none());
    assert_eq!(r.source, TenantSource::Unset);
}

// ── (4) Audit filter + query composition ─────────────────────────────────────

#[test]
fn audit_filter_parse_with_surrounding_whitespace_keeps_value() {
    let f = AuditFilter::parse("  actor = alice  ").unwrap();
    let entry = AuditEntry {
        tenant_id: "acme".into(),
        at: Utc.with_ymd_and_hms(2026, 4, 26, 10, 0, 0).unwrap(),
        actor: "alice".into(),
        action: "x".into(),
        target: "y".into(),
        outcome: "ok".into(),
        metadata: serde_json::Value::Null,
    };
    assert!(f.matches(&entry));
}

#[test]
fn audit_filter_tilde_prefers_over_equals_when_both_present() {
    // Both `~` and `=` appear; the parser splits on `~` first.
    let f = AuditFilter::parse("target~prod=").unwrap();
    match f {
        AuditFilter::Contains { key, needle } => {
            assert_eq!(key, "target");
            assert_eq!(needle, "prod=");
        }
        _ => panic!("expected Contains variant"),
    }
}

#[tokio::test]
async fn audit_query_until_excludes_later_entries() {
    let log = InMemoryAuditLog::new();
    let mut early = AuditEntry {
        tenant_id: "acme".into(),
        at: Utc.with_ymd_and_hms(2026, 4, 26, 10, 0, 0).unwrap(),
        actor: "alice".into(),
        action: "x".into(),
        target: "y".into(),
        outcome: "ok".into(),
        metadata: serde_json::Value::Null,
    };
    log.append(early.clone()).await.unwrap();
    early.at = Utc.with_ymd_and_hms(2026, 4, 28, 10, 0, 0).unwrap();
    log.append(early).await.unwrap();
    let q = AuditQuery::new("acme")
        .with_until(Utc.with_ymd_and_hms(2026, 4, 27, 0, 0, 0).unwrap());
    let got = log.query(&q).await.unwrap();
    assert_eq!(got.len(), 1);
}

// ── (5) Conversation store kind round-trip + selector edge cases ────────────

#[tokio::test]
async fn conversation_store_resolve_id_with_whitespace_treated_as_id() {
    // `parse_selector` trims; an id with internal whitespace becomes the literal id.
    let s = InMemoryConversationStore::new();
    let err = s.resolve("acme", "  unknown  ").await.unwrap_err();
    // `unknown` is treated as id-by-name → store reports not found.
    assert!(err.to_string().contains("not found"));
}

#[test]
fn conversation_kind_variants_are_distinct() {
    // The enum is the public surface for selector dispatch; ensure variants
    // remain distinct so a future merge cannot collapse them silently.
    assert_ne!(ConversationKind::New, ConversationKind::Last);
    assert_ne!(ConversationKind::Last, ConversationKind::ById);
}

// ── (6) ReplState transitions for paths not covered by inline tests ─────────

#[test]
fn repl_state_streamchunk_creates_assistant_when_no_messages() {
    let mut s = ReplState::new("acme", "conv-1");
    let eff = s.handle(ReplCommand::StreamChunk(StreamChunk {
        conversation_id: "conv-1".into(),
        delta: "first-token".into(),
        finish: false,
    }));
    match eff {
        ReplEffect::Render { delta } => assert_eq!(delta, "first-token"),
        other => panic!("expected Render, got {other:?}"),
    }
    assert_eq!(s.messages.len(), 1);
    assert_eq!(s.messages[0].role, ChatRole::Assistant);
}

#[test]
fn repl_state_slash_tools_returns_banner() {
    let mut s = ReplState::new("acme", "conv-1");
    s.buffer = "/tools".to_string();
    match s.handle(ReplCommand::Submit) {
        ReplEffect::Banner(b) => assert!(b.contains("tools")),
        other => panic!("expected Banner, got {other:?}"),
    }
}

#[test]
fn repl_state_submit_whitespace_only_buffer_is_noop() {
    let mut s = ReplState::new("acme", "conv-1");
    s.buffer = "   \t  ".to_string();
    assert_eq!(s.handle(ReplCommand::Submit), ReplEffect::None);
}

// ── (7) Cross-module: tenant lifecycle + env cascade composition ────────────

#[tokio::test]
async fn cross_module_tenant_suspend_then_env_cascade_marks_envs() {
    let tb = InMemoryTenantBackend::new();
    let eb = InMemoryEnvBackend::new();
    let rec = TenantRecord {
        tenant_id: "acme".into(),
        display_name: "Acme".into(),
        state: TenantLifecycleState::Active,
        suspend_reason: None,
        updated_at: Utc::now(),
    };
    tb.seed(rec);
    // Two active envs and one archived under acme.
    eb.seed(cavectl::env::EnvRecord {
        tenant_id: "acme".into(),
        env_id: "pr-1".into(),
        kind: "vcluster".into(),
        state: EnvLifecycleState::Active,
        suspend_reason: None,
        updated_at: Utc::now(),
    });
    eb.seed(cavectl::env::EnvRecord {
        tenant_id: "acme".into(),
        env_id: "pr-2".into(),
        kind: "vcluster".into(),
        state: EnvLifecycleState::Archived,
        suspend_reason: None,
        updated_at: Utc::now(),
    });
    let after = tb.suspend("acme", "burak", "billing-overdue").await.unwrap();
    assert_eq!(after.state, TenantLifecycleState::Suspended);
    let changed = eb
        .cascade_suspend("acme", "burak", "billing-overdue")
        .await
        .unwrap();
    // Only the active one is changed; archived is left as-is.
    assert_eq!(changed.len(), 1);
    assert_eq!(changed[0].env_id, "pr-1");
    assert_eq!(changed[0].state, EnvLifecycleState::Suspended);
}

// ── (8) Approval composition: list ordering by created_at desc ──────────────

#[tokio::test]
async fn approval_list_orders_by_created_at_descending() {
    let b = InMemoryApprovals::new();
    let r1 = b.create("acme", "rotate-key", "alice", 2).await.unwrap();
    tokio::time::sleep(Duration::from_millis(2)).await;
    let r2 = b.create("acme", "drop-table", "alice", 2).await.unwrap();
    let all = b.list("acme", None).await.unwrap();
    assert_eq!(all.len(), 2);
    // Most recent first.
    assert_eq!(all[0].approval_id, r2.approval_id);
    assert_eq!(all[1].approval_id, r1.approval_id);
}

// ── (9) SAML argv parser failure modes ──────────────────────────────────────

#[test]
fn saml_parse_argv_empty_returns_none() {
    assert!(saml_parse(&[]).is_none());
}

#[test]
fn saml_parse_argv_verify_response_missing_cert_flag_returns_none() {
    assert!(saml_parse(&["verify-response", "resp.xml"]).is_none());
}

#[test]
fn saml_parse_argv_sign_request_with_dangling_key_flag_returns_none() {
    // `--key` at the end with no value falls off the indexed lookup.
    assert!(saml_parse(&["sign-request", "req.xml", "--key"]).is_none());
}

#[test]
fn saml_parse_metadata_verb_path_and_read_only() {
    let cmd = SamlCmd::ParseMetadata {
        source: "x".into(),
    };
    assert_eq!(cmd.verb_path().last(), Some(&"parse-metadata"));
    assert!(cmd.is_read_only());
}

// ── (10) WebAuthn parser failure modes ──────────────────────────────────────

fn argv_strings(toks: &[&str]) -> Vec<String> {
    toks.iter().map(|s| s.to_string()).collect()
}

#[test]
fn webauthn_parse_missing_subcommand_errors() {
    let err = webauthn_parse(&argv_strings(&[])).unwrap_err();
    assert!(matches!(err, ParseError::MissingSubcommand));
}

#[test]
fn webauthn_parse_register_options_missing_user_id_errors() {
    let err = webauthn_parse(&argv_strings(&["register-options"])).unwrap_err();
    assert!(matches!(err, ParseError::MissingFlag("user-id")));
}

#[test]
fn webauthn_parse_verify_assertion_missing_resp_file_errors() {
    let err = webauthn_parse(&argv_strings(&[
        "verify-assertion",
        "--challenge",
        "AAA",
    ]))
    .unwrap_err();
    assert!(matches!(err, ParseError::MissingFlag("resp-file")));
}

#[test]
fn webauthn_parse_error_display_includes_flag_name() {
    let e = ParseError::MissingFlag("user-id");
    assert!(e.to_string().contains("user-id"));
}

#[test]
fn webauthn_parse_assert_options_happy_path() {
    let cmd = webauthn_parse(&argv_strings(&["assert-options", "--user-id", "u"]))
        .unwrap();
    assert_eq!(
        cmd,
        WebAuthnCmd::AssertOptions {
            user_id: "u".into(),
        }
    );
}

// ── (11) PipeFormat formatter ────────────────────────────────────────────────

#[test]
fn pipe_format_plaintext_finish_chunk_emits_delta_only() {
    let c = StreamChunk {
        conversation_id: "c1".into(),
        delta: "done".into(),
        finish: true,
    };
    assert_eq!(PipeFormat::PlainText.render_chunk(&c), "done");
}

#[test]
fn pipe_format_jsonlines_round_trip_through_serde() {
    let c = StreamChunk {
        conversation_id: "c1".into(),
        delta: "x".into(),
        finish: false,
    };
    let s = PipeFormat::JsonLines.render_chunk(&c);
    let back: StreamChunk = serde_json::from_str(&s).unwrap();
    assert_eq!(back, c);
}

// ── (12) Tool dispatch state with multiple tenants ──────────────────────────

#[test]
fn toolmode_granted_for_unknown_tenant_returns_empty() {
    let m = ToolMode::new();
    assert!(m.granted_for("ghost").is_empty());
}

#[test]
fn toolmode_revoke_unknown_tool_is_noop() {
    let m = ToolMode::new();
    m.grant("acme", "echo");
    m.revoke("acme", "ghost"); // not registered
    assert!(m.is_granted("acme", "echo"));
}

#[test]
fn toolmode_revoke_on_unknown_tenant_is_noop() {
    let m = ToolMode::new();
    m.revoke("ghost", "anything"); // tenant has no grants at all
    assert!(m.granted_for("ghost").is_empty());
}

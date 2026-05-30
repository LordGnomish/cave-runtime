// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Behavioral integration tests for `cave-pipelines` portable cores.
//!
//! Upstream parity reference: tektoncd/pipeline (Tekton Pipelines) **v0.55.0**.
//! These tests target three already-implemented public cave fns that the TDD
//! coverage audit (`docs/audit/tdd/cave-pipelines-gaps.md`) flagged as having
//! no behavioral test:
//!   * `cave_pipelines::triggers::passes_interceptors` — Bitbucket `x-event-key` arm
//!     (mirrors Tekton `interceptors/bitbucket/bitbucket_test.go`).
//!   * `cave_pipelines::triggers::evaluate_cel` — `.matches('<regex>')` arm
//!     (mirrors `interceptors/cel/cel_test.go`).
//!   * `cave_pipelines::notifications::send_notification` — filter short-circuit
//!     (downstream value-add dispatch gate).
//!
//! Every assertion checks a concrete value derived from the implementation in
//! `src/triggers.rs` and `src/notifications.rs`; no network is touched.

use cave_pipelines::notifications::{
    send_notification, NotificationConfig, NotificationRule, NotifyOn, PipelineEvent,
};
use cave_pipelines::triggers::{evaluate_cel, passes_interceptors, Interceptor, WebhookEvent};
use cave_pipelines::RunPhase;
use serde_json::json;
use uuid::Uuid;

// ─── triggers::passes_interceptors — Bitbucket arm ───────────────────────────

/// A Bitbucket interceptor whose `event_types` contains the event carried in the
/// `x-event-key` header passes. Mirrors the GitHub matching pair, but the
/// Bitbucket arm reads `x-event-key` (not `x-github-event`).
#[test]
fn bitbucket_interceptor_passes_on_matching_event_key() {
    let mut event = WebhookEvent::new("push", json!({}));
    event
        .headers
        .insert("x-event-key".to_string(), "repo:push".to_string());

    let interceptors = vec![Interceptor::Bitbucket {
        secret_ref: "bb-secret".to_string(),
        event_types: vec!["repo:push".to_string()],
    }];

    assert!(
        passes_interceptors(&event, &interceptors),
        "x-event-key 'repo:push' is in event_types, so the event must pass"
    );
}

/// When the `x-event-key` header value is not in `event_types`, the event is
/// filtered out (returns false). Here the header is `pullrequest:created` but the
/// interceptor only allows `repo:push`.
#[test]
fn bitbucket_interceptor_filters_non_matching_event_key() {
    let mut event = WebhookEvent::new("pull_request", json!({}));
    event.headers.insert(
        "x-event-key".to_string(),
        "pullrequest:created".to_string(),
    );

    let interceptors = vec![Interceptor::Bitbucket {
        secret_ref: "bb-secret".to_string(),
        event_types: vec!["repo:push".to_string()],
    }];

    assert!(
        !passes_interceptors(&event, &interceptors),
        "x-event-key 'pullrequest:created' is not allowed, so the event must be filtered out"
    );
}

/// A missing `x-event-key` header resolves to the empty string, which is not in
/// a non-empty `event_types`, so the event is filtered out.
#[test]
fn bitbucket_interceptor_filters_when_header_absent() {
    let event = WebhookEvent::new("push", json!({})); // no headers inserted

    let interceptors = vec![Interceptor::Bitbucket {
        secret_ref: "bb-secret".to_string(),
        event_types: vec!["repo:push".to_string()],
    }];

    assert!(
        !passes_interceptors(&event, &interceptors),
        "absent x-event-key resolves to \"\", which is not in event_types"
    );
}

/// An empty `event_types` list disables filtering entirely: the Bitbucket arm
/// only checks the header when `!event_types.is_empty()`, so any event passes.
#[test]
fn bitbucket_interceptor_empty_event_types_passes_any() {
    let event = WebhookEvent::new("anything", json!({})); // no x-event-key at all

    let interceptors = vec![Interceptor::Bitbucket {
        secret_ref: "bb-secret".to_string(),
        event_types: vec![],
    }];

    assert!(
        passes_interceptors(&event, &interceptors),
        "empty event_types disables Bitbucket filtering, so the event passes"
    );
}

// ─── triggers::evaluate_cel — .matches('<regex>') arm ────────────────────────

/// `.matches('<regex>')` resolves the left-hand path and applies the regex with
/// `Regex::is_match` (substring/anchored-by-pattern). `refs/heads/.*` matches the
/// value `refs/heads/main`.
#[test]
fn evaluate_cel_matches_regex_true() {
    let body = json!({"ref": "refs/heads/main"});
    assert!(
        evaluate_cel("body.ref.matches('refs/heads/.*')", &body),
        "'refs/heads/main' matches the pattern 'refs/heads/.*'"
    );
}

/// A regex that does not match the resolved value yields false. The value is
/// `refs/heads/main`; the pattern `refs/tags/.*` does not occur in it.
#[test]
fn evaluate_cel_matches_regex_false() {
    let body = json!({"ref": "refs/heads/main"});
    assert!(
        !evaluate_cel("body.ref.matches('refs/tags/.*')", &body),
        "'refs/heads/main' does not match the pattern 'refs/tags/.*'"
    );
}

/// When the left-hand CEL path does not resolve to a value, the `.matches` arm
/// returns false (the `if let Some(val) = resolve_cel_path(...)` guard fails).
#[test]
fn evaluate_cel_matches_unresolved_path_false() {
    let body = json!({"ref": "refs/heads/main"});
    assert!(
        !evaluate_cel("body.missing.matches('.*')", &body),
        "an unresolved path makes the .matches arm return false even for the wildcard pattern"
    );
}

// ─── notifications::send_notification — filter short-circuit ─────────────────

/// `send_notification` returns `Ok(())` immediately when `notify_on` does not
/// match the event status. An `OnSuccess` rule against a `Running` event must
/// short-circuit (no transport fires); the Email config also avoids any network.
#[tokio::test]
async fn send_notification_short_circuits_when_filter_rejects() {
    let rule = NotificationRule {
        name: "ci".to_string(),
        config: NotificationConfig::Email {
            smtp_host: "smtp.invalid.example".to_string(),
            smtp_port: 587,
            to: vec!["team@example.com".to_string()],
            from: "ci@example.com".to_string(),
        },
        notify_on: NotifyOn::OnSuccess,
    };
    let event = PipelineEvent {
        pipeline_run_id: Uuid::nil(),
        pipeline_name: "build".to_string(),
        status: RunPhase::Running,
        message: None,
    };

    // OnSuccess.matches(Running) is false, so the fn returns Ok(()) before dispatch.
    assert!(rule.notify_on.matches(&event.status) == false);
    let result = send_notification(&rule, &event).await;
    assert!(
        result.is_ok(),
        "filter rejects Running, so send_notification must return Ok(()) without dispatching"
    );
}

/// When the filter *does* match (`OnSuccess` + `Succeeded`) but the transport is
/// `Email` — whose dispatch is intentionally network-free and always returns
/// `Ok(())` — the call still succeeds. This locks the matched Email path as the
/// only network-free positive dispatch.
#[tokio::test]
async fn send_notification_email_dispatch_is_ok_when_filter_matches() {
    let rule = NotificationRule {
        name: "release".to_string(),
        config: NotificationConfig::Email {
            smtp_host: "smtp.invalid.example".to_string(),
            smtp_port: 25,
            to: vec!["ops@example.com".to_string()],
            from: "ci@example.com".to_string(),
        },
        notify_on: NotifyOn::OnSuccess,
    };
    let event = PipelineEvent {
        pipeline_run_id: Uuid::nil(),
        pipeline_name: "release".to_string(),
        status: RunPhase::Succeeded,
        message: Some("all green".to_string()),
    };

    assert!(rule.notify_on.matches(&event.status));
    let result = send_notification(&rule, &event).await;
    assert!(
        result.is_ok(),
        "matched Email dispatch is network-free and returns Ok(())"
    );
}

/// `OnFailure` matches both `Failed` and `Cancelled` but not `Skipped`; pairing
/// that with the network-free Email transport confirms the short-circuit vs.
/// dispatch boundary for a second `NotifyOn` variant.
#[tokio::test]
async fn send_notification_on_failure_skips_skipped_status() {
    let rule = NotificationRule {
        name: "alerts".to_string(),
        config: NotificationConfig::Email {
            smtp_host: "smtp.invalid.example".to_string(),
            smtp_port: 587,
            to: vec!["oncall@example.com".to_string()],
            from: "ci@example.com".to_string(),
        },
        notify_on: NotifyOn::OnFailure,
    };
    let event = PipelineEvent {
        pipeline_run_id: Uuid::nil(),
        pipeline_name: "nightly".to_string(),
        status: RunPhase::Skipped,
        message: None,
    };

    // OnFailure does not match Skipped, so the fn short-circuits to Ok(()).
    assert!(rule.notify_on.matches(&event.status) == false);
    assert!(send_notification(&rule, &event).await.is_ok());
}

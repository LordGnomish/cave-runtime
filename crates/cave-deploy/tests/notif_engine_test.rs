// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Integration tests for the in-crate notification engine (template rendering,
//! trigger-condition evaluation, oncePer dedup, delivery retry backoff).
//!
//! Upstream: argoproj/notifications-engine — text/template renderer +
//! `pkg/triggers/service.go` (ConditionResult.Key = `[i].{base64url(sha1(when))}`)
//! + `pkg/controller/state.go` (dedup key `{oncePer}:{trigger}:{key}:{service}:{recipient}`).

use cave_deploy::notif_engine::render_template;
use serde_json::json;

fn app_ctx() -> serde_json::Value {
    json!({
        "app": {
            "metadata": { "name": "guestbook", "namespace": "argocd" },
            "spec": { "project": "default" },
            "status": {
                "sync": { "status": "Synced", "revision": "53e28ff20cc530b9ada2173fbbde64d341c1a1c6" },
                "health": { "status": "Healthy" },
                "operationState": { "phase": "Succeeded" }
            }
        },
        "context": { "argocdUrl": "https://cd.example.com" }
    })
}

// ─── Cycle 1: text/template subset renderer ─────────────────────────────────

#[test]
fn render_plain_text_passthrough() {
    let out = render_template("Application deployed.", &app_ctx()).unwrap();
    assert_eq!(out, "Application deployed.");
}

#[test]
fn render_dotted_field_substitution() {
    let out = render_template(
        "Application {{.app.metadata.name}} is now {{.app.status.sync.status}}.",
        &app_ctx(),
    )
    .unwrap();
    assert_eq!(out, "Application guestbook is now Synced.");
}

#[test]
fn render_missing_key_is_go_no_value() {
    // Go text/template default prints "<no value>" for a missing path.
    let out = render_template("rev={{.app.status.sync.nope}}", &app_ctx()).unwrap();
    assert_eq!(out, "rev=<no value>");
}

#[test]
fn render_pipe_functions_upper_lower_title_trim() {
    let ctx = json!({ "x": "  hello WORLD  " });
    assert_eq!(render_template("{{.x | trim | upper}}", &ctx).unwrap(), "HELLO WORLD");
    assert_eq!(render_template("{{.x | trim | lower}}", &ctx).unwrap(), "hello world");
    assert_eq!(render_template("{{.x | trim | title}}", &ctx).unwrap(), "Hello World");
}

#[test]
fn render_if_else_truthiness() {
    let healthy = json!({ "ok": true, "n": 0, "s": "" });
    assert_eq!(
        render_template("{{if .ok}}up{{else}}down{{end}}", &healthy).unwrap(),
        "up"
    );
    // 0, "", null, missing → falsy
    assert_eq!(
        render_template("{{if .n}}nonzero{{else}}zero{{end}}", &healthy).unwrap(),
        "zero"
    );
    assert_eq!(
        render_template("{{if .s}}filled{{else}}empty{{end}}", &healthy).unwrap(),
        "empty"
    );
    assert_eq!(
        render_template("{{if .missing}}y{{else}}n{{end}}", &healthy).unwrap(),
        "n"
    );
}

#[test]
fn render_nested_if_inside_text() {
    let out = render_template(
        "{{.app.metadata.name}}: {{if .app.status.health.status}}health={{.app.status.health.status}}{{end}}",
        &app_ctx(),
    )
    .unwrap();
    assert_eq!(out, "guestbook: health=Healthy");
}

#[test]
fn render_unterminated_action_errors() {
    assert!(render_template("{{.app.metadata.name", &app_ctx()).is_err());
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Integration tests for the in-crate notification engine (template rendering,
//! trigger-condition evaluation, oncePer dedup, delivery retry backoff).
//!
//! Upstream: argoproj/notifications-engine — text/template renderer +
//! `pkg/triggers/service.go` (ConditionResult.Key = `[i].{base64url(sha1(when))}`)
//! + `pkg/controller/state.go` (dedup key `{oncePer}:{trigger}:{key}:{service}:{recipient}`).

use cave_deploy::notif_engine::{
    condition_key, eval_once_per, eval_when, render_template, sha1_base64url,
};
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

// ─── Cycle 2: trigger condition `when`/`oncePer` evaluation + Key hash ───────

#[test]
fn sha1_base64url_golden_vector() {
    // base64-RawURL(SHA1("test")) — matches notifications-engine `hash()`.
    assert_eq!(sha1_base64url("test"), "qUqP5cyxm6YcTAhz05Hph5gvu9M");
}

#[test]
fn condition_key_matches_upstream_format() {
    // ConditionResult.Key = "[i]." + base64url(sha1(when))  (service.go).
    let when = "app.status.sync.status == 'Synced'";
    assert_eq!(condition_key(0, when), "[0].ly_emq_rLIoVosSccMFB8Nm88m8");
    // index participates
    assert!(condition_key(2, when).starts_with("[2]."));
    // distinct `when` → distinct key
    assert_ne!(condition_key(0, when), condition_key(0, "app.x == 1"));
}

#[test]
fn when_equality_against_app_state() {
    let ctx = app_ctx();
    assert!(eval_when("app.status.sync.status == 'Synced'", &ctx).unwrap());
    assert!(!eval_when("app.status.sync.status == 'OutOfSync'", &ctx).unwrap());
    assert!(eval_when("app.status.sync.status != 'OutOfSync'", &ctx).unwrap());
}

#[test]
fn when_membership_in_list() {
    let ctx = app_ctx();
    assert!(eval_when("app.status.operationState.phase in ['Succeeded', 'Error']", &ctx).unwrap());
    assert!(!eval_when("app.status.operationState.phase in ['Running']", &ctx).unwrap());
}

#[test]
fn when_logical_and_or_not() {
    let ctx = app_ctx();
    assert!(
        eval_when(
            "app.status.sync.status == 'Synced' && app.status.health.status == 'Healthy'",
            &ctx
        )
        .unwrap()
    );
    assert!(
        !eval_when(
            "app.status.sync.status == 'Synced' and app.status.health.status == 'Degraded'",
            &ctx
        )
        .unwrap()
    );
    assert!(
        eval_when(
            "app.status.health.status == 'Degraded' || app.status.sync.status == 'Synced'",
            &ctx
        )
        .unwrap()
    );
    assert!(eval_when("!(app.status.sync.status == 'OutOfSync')", &ctx).unwrap());
    assert!(eval_when("not app.spec.missing", &ctx).unwrap());
}

#[test]
fn when_parens_precedence() {
    let ctx = app_ctx();
    // or binds looser than and: false && false || true == true
    assert!(
        eval_when(
            "app.x == 'no' && app.y == 'no' || app.status.sync.status == 'Synced'",
            &ctx
        )
        .unwrap()
    );
    // parenthesised: false && (false || true) == false
    assert!(
        !eval_when(
            "app.x == 'no' && (app.y == 'no' || app.status.sync.status == 'Synced')",
            &ctx
        )
        .unwrap()
    );
}

#[test]
fn once_per_resolves_to_formatted_value() {
    let ctx = app_ctx();
    assert_eq!(
        eval_once_per("app.status.sync.revision", &ctx).unwrap(),
        "53e28ff20cc530b9ada2173fbbde64d341c1a1c6"
    );
    // missing path → empty `%v` of nil → "<nil>" in Go fmt; we mirror that.
    assert_eq!(eval_once_per("app.status.sync.nope", &ctx).unwrap(), "<nil>");
}

#[test]
fn when_malformed_expression_errors() {
    let ctx = app_ctx();
    assert!(eval_when("app.status.sync.status ==", &ctx).is_err());
}

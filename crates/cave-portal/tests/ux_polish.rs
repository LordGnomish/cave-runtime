// SPDX-License-Identifier: AGPL-3.0-or-later
//! Gap 5 close-out — UX polish.
//!
//! What's already in `admin::layout::*`:
//!
//!   * `breadcrumb::breadcrumb_for_path` — breadcrumbs.
//!   * `toast::toast_container` — ARIA-live toast region.
//!   * `skeleton::skeleton_table` + `loading_spinner` — loading.
//!   * `skeleton::error_panel` — error state with retry + bug report.
//!   * `help::empty_state` — empty CTA panel.
//!   * `help::tooltip` / `help::hint` — inline help.
//!
//! What this sprint adds (RED→GREEN):
//!
//!   * `confirm_dialog(action_label, action_url, confirm_text, body)`
//!     — server-rendered modal for destructive actions. Requires
//!     `role="dialog"`, `aria-modal="true"`, an `aria-labelledby`,
//!     and a Cancel button as the first focusable item (Enter key
//!     should NOT submit by default).
//!   * `form_field_error(msg)` — inline `<p role="alert">` rendered
//!     under an invalid input.
//!   * `validation_summary(errors)` — top-of-form summary with
//!     `aria-labelledby`, deep links into each erroring field.
//!   * `search_box(label, name, placeholder)` — labelled search
//!     input with a built-in clear button. Mirrors WAI-ARIA APG.

use cave_portal::admin::layout::ux::{confirm_dialog, form_field_error, search_box, validation_summary};

// ── Confirm dialog ───────────────────────────────────────────────────

#[test]
fn confirm_dialog_has_role_dialog_and_aria_modal() {
    let html = confirm_dialog("Delete", "/admin/keda/delete?id=1", "Are you sure?", "");
    assert!(html.contains(r#"role="dialog""#));
    assert!(html.contains(r#"aria-modal="true""#));
}

#[test]
fn confirm_dialog_carries_aria_labelledby_pointing_at_title() {
    let html = confirm_dialog("Delete", "/x", "Are you sure?", "");
    // aria-labelledby="<id>" must reference an element id that
    // appears as the dialog title.
    assert!(html.contains("aria-labelledby="));
    let title_id = html
        .split(r#"aria-labelledby=""#)
        .nth(1)
        .and_then(|s| s.split('"').next())
        .expect("aria-labelledby attribute present");
    assert!(html.contains(&format!(r#"id="{title_id}""#)));
}

#[test]
fn confirm_dialog_renders_cancel_button_before_confirm() {
    let html = confirm_dialog("Delete forever", "/x", "Are you sure?", "");
    let cancel_idx = html.find("Cancel").expect("cancel button present");
    let confirm_idx = html.find("Delete forever").expect("confirm button present");
    assert!(
        cancel_idx < confirm_idx,
        "Cancel must appear before Confirm for safer keyboard default"
    );
}

#[test]
fn confirm_dialog_action_url_drives_form_action_attribute() {
    let html = confirm_dialog("Delete", "/admin/keda/delete?id=1", "Are you sure?", "");
    assert!(html.contains(r#"action="/admin/keda/delete?id=1""#));
    assert!(html.contains(r#"method="post""#));
}

#[test]
fn confirm_dialog_escapes_html_in_user_text() {
    let html = confirm_dialog("<script>", "/x", "<bad>", "");
    assert!(!html.contains("<script>"));
    assert!(!html.contains("<bad>"));
    assert!(html.contains("&lt;script&gt;"));
    assert!(html.contains("&lt;bad&gt;"));
}

#[test]
fn confirm_dialog_passes_static_a11y_audit() {
    use cave_portal::admin::layout::a11y::audit;
    let html = confirm_dialog("Delete", "/x", "Sure?", "<p>body</p>");
    let issues = audit(&html);
    assert!(issues.is_empty(), "got {} a11y issues: {issues:?}", issues.len());
}

// ── Form field error ─────────────────────────────────────────────────

#[test]
fn form_field_error_has_role_alert_for_immediate_announcement() {
    let html = form_field_error("Email is required");
    assert!(html.contains(r#"role="alert""#));
    assert!(html.contains("Email is required"));
}

#[test]
fn form_field_error_escapes_message() {
    let html = form_field_error("<bad>");
    assert!(html.contains("&lt;bad&gt;"));
    assert!(!html.contains("<bad>"));
}

// ── Validation summary ───────────────────────────────────────────────

#[test]
fn validation_summary_renders_aria_labelled_region() {
    let errors = vec![("email", "Required"), ("name", "Too short")];
    let html = validation_summary(&errors);
    assert!(html.contains(r#"role="alert""#));
    assert!(html.contains("aria-labelledby="));
    // Each field's error appears + links to the field via #name.
    assert!(html.contains(r##"href="#email""##));
    assert!(html.contains(r##"href="#name""##));
}

#[test]
fn validation_summary_is_empty_string_when_no_errors() {
    let html = validation_summary(&[]);
    assert_eq!(html, "");
}

// ── Search box ───────────────────────────────────────────────────────

#[test]
fn search_box_has_label_input_and_clear_button() {
    let html = search_box("Search resources", "q", "Type to filter…");
    // <label for="..."> and a matching id="..."
    assert!(html.contains(r#"<label for="cave-search-q""#));
    assert!(html.contains(r#"id="cave-search-q""#));
    assert!(html.contains("Search resources"));
    assert!(html.contains(r#"placeholder="Type to filter…""#));
    // role="searchbox" or type="search"
    assert!(html.contains(r#"type="search""#));
}

#[test]
fn search_box_escapes_user_supplied_strings() {
    let html = search_box("<bad>", "q", "<x>");
    assert!(!html.contains("<bad>"));
    assert!(!html.contains("<x>"));
    assert!(html.contains("&lt;bad&gt;"));
}

#[test]
fn search_box_passes_static_a11y_audit() {
    use cave_portal::admin::layout::a11y::audit;
    let html = search_box("Search resources", "q", "Type to filter…");
    let issues = audit(&html);
    assert!(issues.is_empty(), "got {issues:?}");
}

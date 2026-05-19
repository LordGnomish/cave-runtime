// SPDX-License-Identifier: AGPL-3.0-or-later
//! UX polish — confirm dialogs, form-field errors, validation
//! summary panels, search box.
//!
//! Gap-5 of the 2026-05-18 user-friendly-and-secure sprint. These
//! widgets fill in the small but high-frequency gaps that the
//! existing helpers (toast, breadcrumb, skeleton, empty_state,
//! tooltip) don't cover.
//!
//! All helpers are server-rendered, accessibility-clean per the
//! existing `admin::layout::a11y::audit` rules, and escape every
//! user-supplied string.

use crate::admin::render::escape;
use std::sync::atomic::{AtomicU64, Ordering};

static UNIQUE: AtomicU64 = AtomicU64::new(1);

fn unique_id(prefix: &str) -> String {
    let n = UNIQUE.fetch_add(1, Ordering::Relaxed);
    format!("cave-{prefix}-{n}")
}

/// Render a confirm-action modal: title prompt + Cancel + dangerous
/// primary action. Cancel sits *before* the destructive button so a
/// keyboard user pressing `Enter` doesn't accidentally trigger the
/// destructive action — they must Tab past Cancel first.
///
/// `action_label` is the visible text + accessible name of the
/// destructive button. `action_url` is the POST target. `prompt` is
/// shown as the modal title. `body_html` is interpolated raw and
/// MUST be pre-escaped by the caller (it's typically a few lines of
/// safe HTML describing what will be deleted).
pub fn confirm_dialog(action_label: &str, action_url: &str, prompt: &str, body_html: &str) -> String {
    let title_id = unique_id("confirm-title");
    let dialog_id = unique_id("confirm");
    format!(
        r##"<div id="{dialog_id}" role="dialog" aria-modal="true" aria-labelledby="{title_id}" aria-label="{prompt_e}" hidden
     class="fixed inset-0 z-40 flex items-center justify-center bg-black/30">
  <form method="post" action="{action_e}"
        class="bg-white dark:bg-zinc-900 rounded shadow-lg w-full max-w-md mx-4 p-5 border dark:border-zinc-700">
    <h2 id="{title_id}" class="text-base font-semibold mb-2">{prompt_e}</h2>
    <div class="text-sm text-zinc-600 dark:text-zinc-300 mb-4">{body}</div>
    <div class="flex justify-end gap-2">
      <button type="button" onclick="document.getElementById('{dialog_id}').hidden=true"
              class="px-3 py-1.5 rounded border text-sm dark:border-zinc-700 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-blue-400">Cancel</button>
      <button type="submit" class="px-3 py-1.5 rounded bg-red-600 hover:bg-red-700 text-white text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-red-400">{action_e_label}</button>
    </div>
  </form>
</div>"##,
        dialog_id = dialog_id,
        title_id = title_id,
        prompt_e = escape(prompt),
        action_e = escape(action_url),
        action_e_label = escape(action_label),
        body = body_html,
    )
}

/// Render an inline error message displayed under an invalid form
/// field. `role="alert"` lets a screen reader announce the message
/// immediately when it appears.
pub fn form_field_error(message: &str) -> String {
    format!(
        r#"<p role="alert" class="mt-1 text-xs text-red-700 dark:text-red-300">{}</p>"#,
        escape(message),
    )
}

/// Render the top-of-form validation summary. Returns the empty
/// string if there are no errors.
pub fn validation_summary(errors: &[(&str, &str)]) -> String {
    if errors.is_empty() {
        return String::new();
    }
    let title_id = unique_id("vsum-title");
    let mut items = String::new();
    for (field, msg) in errors {
        items.push_str(&format!(
            r##"<li><a href="#{f}" class="underline">{m}</a></li>"##,
            f = escape(field),
            m = escape(msg),
        ));
    }
    format!(
        r##"<div role="alert" aria-labelledby="{title_id}"
     class="border border-red-300 dark:border-red-700 bg-red-50 dark:bg-red-900/30 rounded p-3 mb-3 text-sm text-red-900 dark:text-red-100">
  <h3 id="{title_id}" class="font-semibold mb-1">Please fix the following:</h3>
  <ul class="list-disc list-inside space-y-0.5">{items}</ul>
</div>"##,
        title_id = title_id,
        items = items,
    )
}

/// Server-rendered search input with label + clear button.
pub fn search_box(label: &str, name: &str, placeholder: &str) -> String {
    let safe_name = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .collect::<String>();
    let input_id = format!("cave-search-{safe_name}");
    format!(
        r#"<div class="relative">
  <label for="{input_id}" class="block text-xs font-medium text-zinc-600 dark:text-zinc-300 mb-1">{label_e}</label>
  <input id="{input_id}" type="search" name="{name_attr}" placeholder="{placeholder_e}"
         class="w-full px-3 py-1.5 rounded border bg-white dark:bg-zinc-900 dark:border-zinc-700 text-sm focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-400">
</div>"#,
        input_id = input_id,
        label_e = escape(label),
        name_attr = escape(&safe_name),
        placeholder_e = escape(placeholder),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confirm_dialog_basic_structure() {
        let html = confirm_dialog("Delete", "/x", "Sure?", "");
        assert!(html.contains(r#"role="dialog""#));
        assert!(html.contains(r#"aria-modal="true""#));
        assert!(html.contains("Cancel"));
        assert!(html.contains("Delete"));
    }

    #[test]
    fn form_field_error_role_alert() {
        let html = form_field_error("x");
        assert!(html.contains(r#"role="alert""#));
    }

    #[test]
    fn validation_summary_empty_when_no_errors() {
        assert_eq!(validation_summary(&[]), "");
    }

    #[test]
    fn validation_summary_emits_one_li_per_error() {
        let html = validation_summary(&[("a", "msg-a"), ("b", "msg-b")]);
        assert_eq!(html.matches("<li>").count(), 2);
    }

    #[test]
    fn search_box_label_and_input_share_id() {
        let html = search_box("Find", "q", "...");
        assert!(html.contains(r#"<label for="cave-search-q""#));
        assert!(html.contains(r#"id="cave-search-q""#));
    }

    #[test]
    fn search_box_name_attribute_is_sanitised() {
        let html = search_box("Find", "q<script>", "...");
        // Non-alnum chars stripped from name attr and id.
        assert!(html.contains(r#"id="cave-search-qscript""#));
        assert!(!html.contains("<script>"));
    }
}

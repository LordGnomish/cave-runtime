// SPDX-License-Identifier: AGPL-3.0-or-later
//! WCAG 2.1 AA static analyser for server-rendered admin HTML.
//!
//! 2026-05-15 polish sweep — checks the rendered chrome + known admin
//! pages for the most common screen-reader / keyboard-only failures
//! that survive a DOM-text grep:
//!
//!   * **A11y-001** — interactive element with no accessible name
//!     (an `<a>` / `<button>` whose text content + `aria-label` +
//!     `title` are all empty).
//!   * **A11y-002** — `<input>` / `<textarea>` / `<select>` with no
//!     label association (no surrounding `<label>`, no
//!     `aria-label`, no `aria-labelledby`, no `placeholder`).
//!   * **A11y-003** — `<img>` without `alt=`.
//!   * **A11y-004** — modal dialog (`<div ... id="cave-XXX" ...>`
//!     bound to a `Cmd+K` / `?` keypress) without `role="dialog"` +
//!     `aria-modal="true"` + `aria-label=`.
//!   * **A11y-005** — focus-visible affordance missing entirely
//!     from a stylesheet that ships interactive elements.
//!
//! These are deliberately conservative regex passes, not a full
//! tree walk — they catch the regressions that the existing
//! `assert!(html.contains(...))` tests don't, while staying fast
//! enough to run as a unit test on every page render.
//!
//! Usage:
//!
//! ```ignore
//! use crate::admin::layout::a11y::audit;
//! let issues = audit(&shell_v2(opts));
//! assert!(issues.is_empty(), "{:?}", issues);
//! ```

use std::fmt;

/// One WCAG 2.1 AA violation found in a server-rendered HTML chunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct A11yIssue {
    pub code: A11yCode,
    /// Short snippet of the offending element (truncated to 80 chars).
    pub snippet: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum A11yCode {
    /// A11y-001 — interactive element with no accessible name.
    InteractiveWithoutName,
    /// A11y-002 — form control without an associated label.
    InputWithoutLabel,
    /// A11y-003 — `<img>` without `alt=`.
    ImageWithoutAlt,
    /// A11y-004 — modal dialog missing `role`/`aria-modal`/`aria-label`.
    DialogWithoutAria,
    /// A11y-005 — interactive markup ships without any
    /// `:focus-visible` rule, leaving keyboard users without a
    /// caret indicator.
    NoFocusVisibleStyles,
}

impl fmt::Display for A11yCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            A11yCode::InteractiveWithoutName => "A11y-001 InteractiveWithoutName",
            A11yCode::InputWithoutLabel => "A11y-002 InputWithoutLabel",
            A11yCode::ImageWithoutAlt => "A11y-003 ImageWithoutAlt",
            A11yCode::DialogWithoutAria => "A11y-004 DialogWithoutAria",
            A11yCode::NoFocusVisibleStyles => "A11y-005 NoFocusVisibleStyles",
        })
    }
}

/// Scan a rendered HTML document for WCAG AA violations.
pub fn audit(html: &str) -> Vec<A11yIssue> {
    let mut out = Vec::new();
    out.extend(scan_interactive_without_name(html));
    out.extend(scan_inputs_without_label(html));
    out.extend(scan_images_without_alt(html));
    out.extend(scan_dialogs_without_aria(html));
    out.extend(scan_focus_visible(html));
    out
}

fn snippet(s: &str) -> String {
    let trimmed: String = s.chars().take(80).collect();
    if s.chars().count() > 80 {
        format!("{trimmed}…")
    } else {
        trimmed
    }
}

/// A11y-001 — `<button>` / `<a>` with no accessible name.
///
/// Approach: walk through the HTML once, finding each `<button` /
/// `<a` opening tag, capturing the attributes up to the matching
/// `>`, then capturing inner text up to the closing `</button>` /
/// `</a>`. If the element has no text content, no `aria-label`,
/// and no `title`, flag it.
fn scan_interactive_without_name(html: &str) -> Vec<A11yIssue> {
    let mut out = Vec::new();
    for (open, close) in [("<button", "</button>"), ("<a ", "</a>"), ("<a\n", "</a>")] {
        let mut idx = 0;
        while let Some(start) = html[idx..].find(open) {
            let abs_start = idx + start;
            let after_tag = abs_start + open.len();
            let Some(tag_end_rel) = html[after_tag..].find('>') else {
                idx = after_tag;
                continue;
            };
            let tag_end = after_tag + tag_end_rel;
            let attrs = &html[after_tag..tag_end];
            let body_start = tag_end + 1;
            let Some(close_rel) = html[body_start..].find(close) else {
                idx = body_start;
                continue;
            };
            let body = &html[body_start..body_start + close_rel];

            // Strip nested tags from body to get the visible text.
            let visible = strip_tags(body);
            let visible_trimmed = visible.trim();
            let has_aria = attrs.contains("aria-label=") || attrs.contains("aria-labelledby=");
            let has_title = attrs.contains("title=");
            // Self-closing or icon-only without a name → flag.
            if visible_trimmed.is_empty() && !has_aria && !has_title {
                out.push(A11yIssue {
                    code: A11yCode::InteractiveWithoutName,
                    snippet: snippet(&html[abs_start..body_start + close_rel + close.len()]),
                });
            }
            idx = body_start + close_rel + close.len();
        }
    }
    out
}

fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

/// A11y-002 — `<input>` / `<textarea>` / `<select>` without label.
fn scan_inputs_without_label(html: &str) -> Vec<A11yIssue> {
    let mut out = Vec::new();
    for tag in ["<input", "<textarea", "<select"] {
        let mut idx = 0;
        while let Some(start) = html[idx..].find(tag) {
            let abs_start = idx + start;
            let after_tag = abs_start + tag.len();
            // Make sure this is a tag opening, not e.g. <inputfoo
            let next_char = html[after_tag..].chars().next();
            if !matches!(next_char, Some(' ') | Some('\n') | Some('\t') | Some('>')) {
                idx = after_tag;
                continue;
            }
            let Some(tag_end_rel) = html[after_tag..].find('>') else {
                idx = after_tag;
                continue;
            };
            let tag_end = after_tag + tag_end_rel;
            let attrs = &html[after_tag..tag_end];

            // Hidden inputs don't need a label — they're for state passing.
            if attrs.contains(r#"type="hidden""#) {
                idx = tag_end + 1;
                continue;
            }
            // Inputs with type=submit/reset/button typically carry a value=.
            if (attrs.contains(r#"type="submit""#)
                || attrs.contains(r#"type="reset""#)
                || attrs.contains(r#"type="button""#))
                && attrs.contains("value=")
            {
                idx = tag_end + 1;
                continue;
            }

            let has_aria = attrs.contains("aria-label=") || attrs.contains("aria-labelledby=");
            let has_placeholder = attrs.contains("placeholder=");
            // Best-effort `<label>` association: surrounding markup
            // looks like `<label> ... <input ...> ... </label>`.
            // We approximate by searching backwards 200 chars for an
            // unclosed `<label`.
            let lookback_start = abs_start.saturating_sub(200);
            let lookback = &html[lookback_start..abs_start];
            let in_label = lookback.rfind("<label").map_or(false, |li| {
                lookback[li..].find("</label>").is_none()
            });

            if !has_aria && !has_placeholder && !in_label {
                out.push(A11yIssue {
                    code: A11yCode::InputWithoutLabel,
                    snippet: snippet(&html[abs_start..=tag_end]),
                });
            }
            idx = tag_end + 1;
        }
    }
    out
}

/// A11y-003 — `<img>` without `alt=`.
fn scan_images_without_alt(html: &str) -> Vec<A11yIssue> {
    let mut out = Vec::new();
    let mut idx = 0;
    while let Some(start) = html[idx..].find("<img") {
        let abs_start = idx + start;
        let after_tag = abs_start + "<img".len();
        let Some(tag_end_rel) = html[after_tag..].find('>') else {
            idx = after_tag;
            continue;
        };
        let tag_end = after_tag + tag_end_rel;
        let attrs = &html[after_tag..tag_end];
        if !attrs.contains("alt=") {
            out.push(A11yIssue {
                code: A11yCode::ImageWithoutAlt,
                snippet: snippet(&html[abs_start..=tag_end]),
            });
        }
        idx = tag_end + 1;
    }
    out
}

/// A11y-004 — modal `<div ... hidden ...>` patterns without ARIA.
///
/// Scans for `<div` opening tags whose attributes carry both
/// `id="cave-..."` and the literal `hidden` (our modal pattern).
/// Each must carry `role="dialog"`, `aria-modal="true"`, and an
/// `aria-label`.
fn scan_dialogs_without_aria(html: &str) -> Vec<A11yIssue> {
    let mut out = Vec::new();
    let mut idx = 0;
    while let Some(start) = html[idx..].find("<div") {
        let abs_start = idx + start;
        let after_tag = abs_start + "<div".len();
        let Some(tag_end_rel) = html[after_tag..].find('>') else {
            idx = after_tag;
            continue;
        };
        let tag_end = after_tag + tag_end_rel;
        let attrs = &html[after_tag..tag_end];

        let looks_like_modal = attrs.contains(r#"id="cave-"#) && attrs.contains(" hidden");
        if looks_like_modal {
            let has_role = attrs.contains(r#"role="dialog""#);
            let has_modal = attrs.contains(r#"aria-modal="true""#);
            let has_label = attrs.contains("aria-label=");
            if !(has_role && has_modal && has_label) {
                out.push(A11yIssue {
                    code: A11yCode::DialogWithoutAria,
                    snippet: snippet(&html[abs_start..=tag_end]),
                });
            }
        }
        idx = tag_end + 1;
    }
    out
}

/// A11y-005 — interactive markup without any `:focus-visible` rule.
fn scan_focus_visible(html: &str) -> Vec<A11yIssue> {
    let has_interactive = html.contains("<button") || html.contains("<a ") || html.contains("<input");
    if !has_interactive {
        return Vec::new();
    }
    if html.contains(":focus-visible") || html.contains("focus-visible:") {
        return Vec::new();
    }
    vec![A11yIssue {
        code: A11yCode::NoFocusVisibleStyles,
        snippet: "(document carries interactive elements but no focus-visible rule)".into(),
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── A11y-001 — InteractiveWithoutName ─────────────────────────

    #[test]
    fn flags_button_with_empty_body_and_no_aria() {
        let html = r#"<button class="x"></button>"#;
        let issues = audit(html);
        assert!(issues.iter().any(|i| i.code == A11yCode::InteractiveWithoutName));
    }

    #[test]
    fn passes_button_with_text() {
        let html = r#"<button class="x">Save</button>"#;
        let issues = audit(html);
        assert!(!issues.iter().any(|i| i.code == A11yCode::InteractiveWithoutName));
    }

    #[test]
    fn passes_icon_only_button_with_aria_label() {
        let html = r#"<button aria-label="Close"><svg></svg></button>"#;
        let issues = audit(html);
        assert!(!issues.iter().any(|i| i.code == A11yCode::InteractiveWithoutName));
    }

    #[test]
    fn passes_anchor_with_text_inside_nested_span() {
        let html = r#"<a href="/x"><span>Go</span></a>"#;
        let issues = audit(html);
        assert!(!issues.iter().any(|i| i.code == A11yCode::InteractiveWithoutName));
    }

    #[test]
    fn flags_anchor_with_only_nested_svg() {
        let html = r#"<a href="/x"><svg></svg></a>"#;
        let issues = audit(html);
        assert!(issues.iter().any(|i| i.code == A11yCode::InteractiveWithoutName));
    }

    // ── A11y-002 — InputWithoutLabel ──────────────────────────────

    #[test]
    fn flags_input_without_label_or_placeholder() {
        let html = r#"<form><input type="text" name="email"></form>"#;
        let issues = audit(html);
        assert!(issues.iter().any(|i| i.code == A11yCode::InputWithoutLabel));
    }

    #[test]
    fn passes_input_with_placeholder() {
        let html = r#"<input type="text" placeholder="email">"#;
        let issues = audit(html);
        assert!(!issues.iter().any(|i| i.code == A11yCode::InputWithoutLabel));
    }

    #[test]
    fn passes_input_with_aria_label() {
        let html = r#"<input type="text" aria-label="search">"#;
        let issues = audit(html);
        assert!(!issues.iter().any(|i| i.code == A11yCode::InputWithoutLabel));
    }

    #[test]
    fn passes_input_inside_label_element() {
        let html = r#"<label>Name<input type="text" name="x"></label>"#;
        let issues = audit(html);
        assert!(!issues.iter().any(|i| i.code == A11yCode::InputWithoutLabel));
    }

    #[test]
    fn ignores_hidden_input() {
        let html = r#"<input type="hidden" name="csrf" value="abc">"#;
        let issues = audit(html);
        assert!(!issues.iter().any(|i| i.code == A11yCode::InputWithoutLabel));
    }

    #[test]
    fn ignores_submit_button_with_value() {
        let html = r#"<input type="submit" value="Save">"#;
        let issues = audit(html);
        assert!(!issues.iter().any(|i| i.code == A11yCode::InputWithoutLabel));
    }

    // ── A11y-003 — ImageWithoutAlt ────────────────────────────────

    #[test]
    fn flags_image_without_alt() {
        let html = r#"<img src="/logo.png">"#;
        let issues = audit(html);
        assert!(issues.iter().any(|i| i.code == A11yCode::ImageWithoutAlt));
    }

    #[test]
    fn passes_image_with_empty_alt_decorative() {
        let html = r#"<img src="/decor.png" alt="">"#;
        let issues = audit(html);
        assert!(!issues.iter().any(|i| i.code == A11yCode::ImageWithoutAlt));
    }

    // ── A11y-004 — DialogWithoutAria ──────────────────────────────

    #[test]
    fn flags_modal_without_role_dialog() {
        let html = r#"<div id="cave-foo" hidden class="modal">..</div>"#;
        let issues = audit(html);
        assert!(issues.iter().any(|i| i.code == A11yCode::DialogWithoutAria));
    }

    #[test]
    fn passes_modal_with_full_aria() {
        let html = r#"<div id="cave-help" role="dialog" aria-modal="true" aria-label="Keyboard shortcuts" hidden>..</div>"#;
        let issues = audit(html);
        assert!(!issues.iter().any(|i| i.code == A11yCode::DialogWithoutAria));
    }

    // ── A11y-005 — NoFocusVisibleStyles ───────────────────────────

    #[test]
    fn flags_doc_with_button_but_no_focus_visible() {
        let html = r#"<style>.x{color:red}</style><button>x</button>"#;
        let issues = audit(html);
        assert!(issues.iter().any(|i| i.code == A11yCode::NoFocusVisibleStyles));
    }

    #[test]
    fn passes_doc_with_focus_visible_rule() {
        let html = r#"<style>.x:focus-visible{outline:2px solid blue}</style><button>x</button>"#;
        let issues = audit(html);
        assert!(!issues.iter().any(|i| i.code == A11yCode::NoFocusVisibleStyles));
    }

    #[test]
    fn passes_doc_with_tailwind_focus_visible_utility() {
        let html = r#"<button class="focus-visible:outline-2">x</button>"#;
        let issues = audit(html);
        assert!(!issues.iter().any(|i| i.code == A11yCode::NoFocusVisibleStyles));
    }

    #[test]
    fn no_issues_for_pure_text_document() {
        let html = "<p>hello</p>";
        let issues = audit(html);
        assert!(issues.is_empty());
    }

    // ── Lock-in: rendered chrome must produce ZERO violations ─────

    #[test]
    fn shell_v2_passes_full_a11y_audit() {
        use crate::admin::layout::shell::{shell_v2, ShellOptions};
        use crate::admin::permission::Persona;
        let html = shell_v2(ShellOptions {
            title: "WCAG audit",
            persona: Persona::PlatformAdmin,
            tenant_id: "acme",
            current_path: "/admin/keda",
            theme_cookie: None,
            breadcrumb: None,
            extra_commands: Vec::new(),
            cluster_info: "1 node",
            hide_sidebar: false,
            body: "<p>body</p>",
        });
        let issues = audit(&html);
        assert!(
            issues.is_empty(),
            "shell_v2 must be WCAG AA clean; got {} violations:\n{}",
            issues.len(),
            issues
                .iter()
                .map(|i| format!("  - {}: {}", i.code, i.snippet))
                .collect::<Vec<_>>()
                .join("\n"),
        );
    }

    #[test]
    fn shell_v2_for_tenant_admin_also_passes_a11y_audit() {
        use crate::admin::layout::shell::{shell_v2, ShellOptions};
        use crate::admin::permission::Persona;
        // Same chrome but with TenantAdmin persona — disabled
        // shortcut rows + filtered palette must still be a11y-clean.
        let html = shell_v2(ShellOptions {
            title: "WCAG audit · tenant",
            persona: Persona::TenantAdmin,
            tenant_id: "tenant1",
            current_path: "/admin/keda",
            theme_cookie: None,
            breadcrumb: None,
            extra_commands: Vec::new(),
            cluster_info: "1 node",
            hide_sidebar: false,
            body: "<p>body</p>",
        });
        let issues = audit(&html);
        assert!(
            issues.is_empty(),
            "TenantAdmin chrome must be WCAG AA clean; got: {issues:?}",
        );
    }
}

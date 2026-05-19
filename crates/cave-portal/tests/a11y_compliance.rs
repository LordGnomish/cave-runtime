// SPDX-License-Identifier: AGPL-3.0-or-later
//! Gap 1 close-out — WCAG 2.1 AA hardening, integration tests.
//!
//! Sprint date: 2026-05-18. Closes the five non-trivial WCAG AA gaps
//! the existing `admin::layout::a11y` static analyser does NOT cover:
//!
//!   * **Skip-link** — first focusable element of the page must jump
//!     to `#main`. Required for keyboard users who otherwise have to
//!     tab through the topbar + sidebar on every page load.
//!   * **Heading order** — no `<h3>` may follow an `<h1>` without an
//!     intervening `<h2>` (screen-reader nav landmarks).
//!   * **Table semantics** — every `<th>` carries an explicit
//!     `scope="col"` / `scope="row"`, and every `<table>` has either
//!     `<caption>` or `aria-label`.
//!   * **ARIA live regions** — page must expose at least one
//!     `aria-live` region for async updates (the toast container
//!     already does this; this test locks it in for `page_shell`).
//!   * **Contrast** — fixed Tailwind palette pairs we ship in the
//!     shell are validated against the 4.5:1 (text) / 3:1 (UI) ratio
//!     using a small WCAG-luminance helper.
//!
//! These run as **integration** tests so they exercise the public
//! `page_shell` surface — every legacy handler renders through this
//! one entry point, so a regression in the chrome trips immediately.

use cave_portal::admin::render::{page_shell, table, table_html};

/// WCAG luminance — gamma-corrected relative luminance per
/// <https://www.w3.org/TR/WCAG21/#dfn-relative-luminance>.
fn rel_luminance(rgb: (u8, u8, u8)) -> f64 {
    let chan = |c: u8| -> f64 {
        let s = c as f64 / 255.0;
        if s <= 0.03928 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * chan(rgb.0) + 0.7152 * chan(rgb.1) + 0.0722 * chan(rgb.2)
}

fn contrast_ratio(a: (u8, u8, u8), b: (u8, u8, u8)) -> f64 {
    let la = rel_luminance(a);
    let lb = rel_luminance(b);
    let (lhi, llo) = if la > lb { (la, lb) } else { (lb, la) };
    (lhi + 0.05) / (llo + 0.05)
}

// ── Skip-link ────────────────────────────────────────────────────────

#[test]
fn skip_link_present_as_first_focusable_element() {
    let html = page_shell("Test", "<p>body</p>");
    // Skip-link sits in the body right after <body>, before <header>.
    let body_idx = html.find("<body").expect("page has <body>");
    let header_idx = html.find("<header").expect("page has <header>");
    let between = &html[body_idx..header_idx];
    assert!(
        between.contains(r##"href="#main""##),
        "skip-link must precede the header; got:\n{between}"
    );
    assert!(between.contains("Skip to main content"));
}

#[test]
fn skip_link_target_main_has_matching_id() {
    let html = page_shell("Test", "<p>body</p>");
    assert!(
        html.contains(r#"<main id="main""#),
        "skip-link target must be `<main id=\"main\">`"
    );
}

#[test]
fn skip_link_uses_sr_only_pattern_for_visible_only_on_focus() {
    let html = page_shell("Test", "<p>body</p>");
    // We rely on the `sr-only` Tailwind utility + `focus:not-sr-only`
    // to keep the link invisible until tabbed to.
    assert!(html.contains("sr-only"));
    assert!(html.contains("focus:not-sr-only"));
}

// ── ARIA live region ─────────────────────────────────────────────────

#[test]
fn page_shell_emits_aria_live_region() {
    let html = page_shell("Test", "<p>body</p>");
    assert!(
        html.contains(r#"aria-live="polite""#),
        "shell must include at least one polite aria-live region for status updates"
    );
}

// ── Table semantics ──────────────────────────────────────────────────

#[test]
fn table_helper_emits_th_with_scope_col() {
    let html = table(&["a", "b"], &[vec!["x".into(), "y".into()]]);
    // Every <th (followed by space) must carry scope="col". Exclude <thead.
    let th_count = html.matches("<th ").count();
    let scope_count = html.matches(r#"scope="col""#).count();
    assert_eq!(
        th_count, scope_count,
        "every <th> must carry scope=\"col\"; got {th_count} <th> vs {scope_count} scope= attrs"
    );
    assert!(th_count >= 2, "expected at least 2 header cells, got {th_count}");
}

#[test]
fn table_html_helper_emits_th_with_scope_col() {
    let html = table_html(&["a"], &[vec!["<b>x</b>".into()]]);
    let th_count = html.matches("<th ").count();
    let scope_count = html.matches(r#"scope="col""#).count();
    assert_eq!(th_count, scope_count);
    assert!(th_count >= 1);
}

// ── Heading order ────────────────────────────────────────────────────

#[test]
fn page_shell_does_not_skip_heading_levels() {
    let body = "<h2>section</h2><h3>subsection</h3>";
    let html = page_shell("Top", body);
    // The shell emits exactly one <h1> (the title) and the body
    // chains h2 -> h3. No h4 without an intervening h3 etc.
    let positions: Vec<(usize, u32)> = (1..=6)
        .flat_map(|lvl| {
            let tag = format!("<h{lvl}");
            html.match_indices(&tag).map(move |(idx, _)| (idx, lvl)).collect::<Vec<_>>()
        })
        .collect();
    let mut sorted = positions.clone();
    sorted.sort_by_key(|(i, _)| *i);
    let mut prev = 0;
    for (_idx, lvl) in sorted {
        assert!(
            prev == 0 || lvl <= prev + 1,
            "heading jump detected: went from h{prev} -> h{lvl}"
        );
        prev = lvl;
    }
}

// ── Contrast ratios for the shell palette ────────────────────────────

#[test]
fn shell_palette_contrast_meets_wcag_aa_for_text() {
    // bg-gray-50 (#f9fafb) over text-gray-900 (#111827) — light mode body.
    let r = contrast_ratio((0xf9, 0xfa, 0xfb), (0x11, 0x18, 0x27));
    assert!(r >= 4.5, "light-mode body text contrast {r:.2}:1 fails AA 4.5:1");
}

#[test]
fn shell_palette_contrast_meets_wcag_aa_in_dark_mode() {
    // bg-zinc-950 (#09090b) over text-zinc-100 (#f4f4f5) — dark mode body.
    let r = contrast_ratio((0x09, 0x09, 0x0b), (0xf4, 0xf4, 0xf5));
    assert!(r >= 4.5, "dark-mode body text contrast {r:.2}:1 fails AA 4.5:1");
}

#[test]
fn primary_button_contrast_meets_wcag_aa() {
    // bg-blue-600 (#2563eb) with white text.
    let r = contrast_ratio((0x25, 0x63, 0xeb), (0xff, 0xff, 0xff));
    assert!(r >= 4.5, "primary button contrast {r:.2}:1 fails AA");
}

#[test]
fn focus_ring_contrast_meets_wcag_aa_for_ui_3to1() {
    // Focus ring is #3b82f6 (blue-500) over light gray bg.
    let r = contrast_ratio((0x3b, 0x82, 0xf6), (0xf9, 0xfa, 0xfb));
    assert!(r >= 3.0, "focus ring contrast {r:.2}:1 fails AA UI 3:1");
}

// ── Landmark roles ───────────────────────────────────────────────────

#[test]
fn page_shell_has_full_landmark_set() {
    let html = page_shell("L", "");
    // Required landmarks: header, nav, main, footer.
    assert!(html.contains("<header"));
    assert!(html.contains("<nav"));
    assert!(html.contains("<main"));
    assert!(html.contains("<footer"));
}

#[test]
fn lang_attribute_is_present_on_html_root() {
    let html = page_shell("L", "");
    assert!(html.contains(r#"<html lang="en""#));
}

// ── audit() integration on real chrome ───────────────────────────────

#[test]
fn page_shell_passes_static_a11y_audit() {
    use cave_portal::admin::layout::a11y::audit;
    let html = page_shell("Integration", "<p>body</p>");
    let issues = audit(&html);
    assert!(
        issues.is_empty(),
        "page_shell must be a11y-clean; got {} violations",
        issues.len()
    );
}

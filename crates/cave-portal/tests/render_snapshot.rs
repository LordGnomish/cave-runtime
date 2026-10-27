// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Render snapshot tests for cave-portal — structural assertions on
// the HTML emitted by the new UX primitives so regressions in the
// shape of the markup are caught early. No external snapshot crate
// (insta etc.) so the workspace stays dependency-clean; each test
// asserts on a few load-bearing fragments instead of matching the
// full output byte-for-byte.
//
// Run with: cargo test -p cave-portal --test render_snapshot

use cave_portal::admin::render::{
    badge, empty_state, error_state, markdown_lite, search_box, skeleton, sortable_table,
};

#[test]
fn snapshot_empty_state_shape() {
    let html = empty_state("📭", "No data", "Hint goes here.");
    // role + class + structural divs
    assert!(html.contains(r#"role="status""#), "missing role=status: {html}");
    assert!(html.contains(r#"class="cave-empty""#));
    assert!(html.contains(r#"class="cave-empty-icon""#));
    assert!(html.contains(r#"class="cave-empty-title""#));
    assert!(html.contains(r#"class="cave-empty-hint""#));
    // Order: icon → title → hint
    let icon_idx = html.find("cave-empty-icon").unwrap();
    let title_idx = html.find("cave-empty-title").unwrap();
    let hint_idx = html.find("cave-empty-hint").unwrap();
    assert!(icon_idx < title_idx && title_idx < hint_idx);
}

#[test]
fn snapshot_error_state_shape() {
    let html = error_state("Fetch failed", "connection refused");
    assert!(html.contains(r#"role="alert""#));
    assert!(html.contains(r#"class="cave-error""#));
    assert!(html.contains("<strong>Fetch failed:</strong>"));
}

#[test]
fn snapshot_skeleton_emits_aria_busy_and_n_bars() {
    let html = skeleton(3);
    assert!(html.contains(r#"aria-busy="true""#));
    assert!(html.contains(r#"aria-label="Loading""#));
    assert_eq!(html.matches("cave-skeleton").count(), 3);
}

#[test]
fn snapshot_badge_palette_per_tone() {
    let palette = [
        ("ok", "bg-green-100"),
        ("warn", "bg-yellow-100"),
        ("bad", "bg-red-100"),
        ("info", "bg-blue-100"),
        ("neutral", "bg-zinc-100"),
        ("unknown-fallback", "bg-zinc-100"),
    ];
    for (tone, expect_cls) in palette {
        let html = badge(tone, "v");
        assert!(html.contains(expect_cls), "tone={tone} missing {expect_cls}: {html}");
        assert!(html.contains(">v</span>"));
    }
}

#[test]
fn snapshot_search_box_shape() {
    let html = search_box("#tbl", "find…");
    assert!(html.contains(r#"type="search""#));
    assert!(html.contains(r#"class="cave-search""#));
    assert!(html.contains(r#"placeholder="find…""#));
    assert!(html.contains(r##"aria-controls="#tbl""##));
    // Counter pill exists and starts at em-dash (initial state)
    assert!(html.contains(r##"id="cave-search-tbl-count""##));
    assert!(html.contains("showing"));
    assert!(html.contains("rows"));
}

#[test]
fn snapshot_sortable_table_shape() {
    let html = sortable_table(
        "demo",
        &[("first", "text"), ("second", "num")],
        &[
            vec!["row1-a".into(), "<b>row1-b</b>".into()],
            vec!["row2-a".into(), "row2-b".into()],
        ],
    );
    assert!(html.contains(r#"<table id="demo""#));
    assert!(html.contains("data-sortable"));
    assert!(html.contains(r#"data-sort="text""#));
    assert!(html.contains(r#"data-sort="num""#));
    assert!(html.contains(r#"role="button""#));
    assert!(html.contains(r#"tabindex="0""#));
    assert!(html.contains(r#"aria-sort="none""#));
    // Cells rendered verbatim — caller controls HTML.
    assert!(html.contains("<b>row1-b</b>"));
    // Inline script wires sort handlers, idempotent.
    assert!(html.contains("__caveSorted"));
    assert!(html.contains("localeCompare"));
}

#[test]
fn snapshot_markdown_lite_full_grammar() {
    let src = r#"# Heading 1

A paragraph with `code`, **bold**, *em*, and [a link](https://example.com).

## Heading 2

> A quote

- bullet a
- bullet b

1. one
2. two

```
fn main() {}
```

---

End."#;
    let html = markdown_lite(src);

    // Headings
    assert!(html.contains("<h1>Heading 1</h1>"));
    assert!(html.contains("<h2>Heading 2</h2>"));
    // Inline
    assert!(html.contains("<code>code</code>"));
    assert!(html.contains("<strong>bold</strong>"));
    assert!(html.contains("<em>em</em>"));
    assert!(html.contains(r#"<a href="https://example.com" target="_blank" rel="noopener">a link</a>"#));
    // Blocks
    assert!(html.contains("<blockquote>A quote</blockquote>"));
    assert!(html.contains("<ul><li>bullet a</li><li>bullet b</li></ul>"));
    assert!(html.contains("<ol><li>one</li><li>two</li></ol>"));
    assert!(html.contains("<pre><code>fn main() {}"));
    assert!(html.contains("<hr>"));
    // Final paragraph
    assert!(html.contains("<p>End.</p>"));
}

#[test]
fn snapshot_markdown_lite_rejects_dangerous_links() {
    // javascript: and data: URLs must NOT produce anchor tags; they
    // fall through as escaped literal text.
    let html_js = markdown_lite("[xss](javascript:alert(1))");
    assert!(!html_js.contains("<a href=\"javascript:"));
    assert!(html_js.contains("[xss](javascript:alert(1))"));
    let html_data = markdown_lite("[xss](data:text/html;base64,PHNjcmlwdD4=)");
    assert!(!html_data.contains("<a href=\"data:"));
}

/// Render-time bench — must complete in under 50ms for a 200-row table
/// even on cold caches. This is a regression guard, not a perf target.
/// If this fires, look for accidental String::new() inside the row loop
/// or a sort that grew super-linear.
#[test]
fn bench_sortable_table_200_rows_under_50ms() {
    let headers: Vec<(&str, &str)> = vec![
        ("a", "text"),
        ("b", "text"),
        ("c", "num"),
        ("d", "text"),
        ("e", "text"),
    ];
    let rows: Vec<Vec<String>> = (0..200)
        .map(|i| {
            vec![
                format!("crate-{i}"),
                format!("repo/{i}"),
                format!("{}", i % 100),
                format!("ADR-{i:03}"),
                format!("status-{}", i % 5),
            ]
        })
        .collect();
    let start = std::time::Instant::now();
    for _ in 0..10 {
        let html = sortable_table("bench", &headers, &rows);
        // Make sure the compiler doesn't optimise the whole thing away.
        assert!(html.len() > 1000);
    }
    let elapsed = start.elapsed();
    assert!(
        elapsed < std::time::Duration::from_millis(500),
        "10x 200-row sortable_table render took {}ms — investigate",
        elapsed.as_millis()
    );
}

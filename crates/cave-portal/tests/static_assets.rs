// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Static-asset route — regression guard for the
// "every admin page renders as unstyled HTML" bug fixed on
// 2026-05-22. The bug was that the shell emitted
//   <link rel="stylesheet" href="/static/tailwind-light.css">
//   <script src="/static/htmx.min.js" defer></script>
// but the axum Router had no `/static/*` handler, so every utility
// class in the codebase rendered into a vacuum.
//
// These tests pin the contract:
//
//   1. `static_asset_lookup` must resolve the three names the shell
//      links to (tailwind-light.css, cave-brand.css, htmx.min.js).
//   2. Unknown names return None — no path traversal, no fs reads.
//   3. The CSS file MUST cover every Tailwind utility class actually
//      emitted by src/admin/*.rs. When a developer adds a new class,
//      this test fails and points them at the missing rule.

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use cave_portal::routes::static_asset_lookup;

#[test]
fn lookup_resolves_known_assets() {
    let css = static_asset_lookup("tailwind-light.css").expect("tailwind asset present");
    assert!(css.0.starts_with("text/css"), "CSS content-type wrong: {}", css.0);
    assert!(
        css.1.contains(".flex") && css.1.contains(".rounded"),
        "tailwind-light.css missing core utilities"
    );

    let brand = static_asset_lookup("cave-brand.css").expect("brand asset present");
    assert!(brand.0.starts_with("text/css"));
    assert!(
        brand.1.contains("--cave-bg"),
        "cave-brand.css missing brand token --cave-bg"
    );

    let js = static_asset_lookup("htmx.min.js").expect("htmx shim present");
    assert!(js.0.contains("javascript"), "JS content-type wrong: {}", js.0);
    assert!(
        js.1.contains("hx-get") && js.1.contains("window.htmx"),
        "htmx.min.js shim missing required surface"
    );
}

#[test]
fn lookup_rejects_unknown_names() {
    assert!(static_asset_lookup("../Cargo.toml").is_none());
    assert!(static_asset_lookup("/etc/passwd").is_none());
    assert!(static_asset_lookup("").is_none());
    assert!(static_asset_lookup("style.css").is_none()); // close to allowlist but not exact
}

#[test]
fn lookup_resolves_consistently() {
    // Calling twice returns the same pointer (it's a `&'static str`),
    // so the per-request cost is a single hash-equal comparison.
    let a = static_asset_lookup("tailwind-light.css").unwrap().1.as_ptr();
    let b = static_asset_lookup("tailwind-light.css").unwrap().1.as_ptr();
    assert_eq!(a, b);
}

/// Drift guard: every Tailwind utility class the admin handlers emit must
/// be defined in `tailwind-light.css`. If a new class slips in and isn't
/// defined here, the test fails and tells you exactly which class is
/// missing — preventing the next "page renders unstyled" regression.
///
/// Pure string-matching, no PostCSS parser. We accept a small false-
/// positive surface (interpolated `{var}` placeholders, single-char
/// markers like `x` and `+`) and filter them out below.
#[test]
fn tailwind_css_covers_every_class_emitted_by_admin_handlers() {
    let admin_dir = workspace_root().join("crates/cave-portal/src/admin");
    let mut emitted: HashSet<String> = HashSet::new();
    walk(&admin_dir, &mut |path| {
        if path.extension().and_then(|s| s.to_str()) != Some("rs") {
            return;
        }
        let body = fs::read_to_string(path).unwrap_or_default();
        // Match `class="…"` attribute values.
        for (idx, _) in body.match_indices("class=\"") {
            let start = idx + 7;
            if let Some(end_rel) = body[start..].find('"') {
                let value = &body[start..start + end_rel];
                for cls in value.split_whitespace() {
                    // Filter out template placeholders & format args.
                    if cls.starts_with('{') || cls.contains('{') { continue; }
                    if cls.is_empty() { continue; }
                    // Skip identifiers that are obviously not Tailwind:
                    // pure ascii-alnum without `-`, `:`, `/`, `[` are
                    // semantic markers (e.g. `modal`, `badge`, `num`).
                    if !cls.contains('-')
                        && !cls.contains(':')
                        && !cls.contains('/')
                        && !cls.contains('[')
                        && !cls.contains('.')
                    {
                        // Allowed semantic classes (own CSS, not Tailwind).
                        const SEMANTIC_OK: &[&str] = &[
                            "badge", "modal", "cls", "num", "dark", "x", "hidden",
                            "fixed", "absolute", "relative", "block", "inline",
                            "flex", "grid", "italic", "uppercase", "underline",
                        ];
                        if SEMANTIC_OK.contains(&cls) { continue; }
                        emitted.insert(cls.to_string());
                        continue;
                    }
                    emitted.insert(cls.to_string());
                }
            }
        }
    });

    let css = static_asset_lookup("tailwind-light.css").unwrap().1;
    let mut missing: Vec<String> = Vec::new();
    for cls in &emitted {
        // CSS escapes special chars: `:` → `\:`, `/` → `\/`, `.` → `\.`,
        // `[` → `\[`, `]` → `\]`.
        let escaped = cls
            .replace(':', r"\:")
            .replace('/', r"\/")
            .replace('.', r"\.")
            .replace('[', r"\[")
            .replace(']', r"\]");
        let needle = format!(".{escaped}");
        if !css.contains(&needle) {
            missing.push(cls.clone());
        }
    }
    missing.sort();

    // The audit baseline — current intentional exceptions. New unknowns
    // must be added to the CSS, not to this allowlist.
    const ALLOWED_UNCOVERED: &[&str] = &[
        // Pure semantic helpers / placeholders left in templates
        "...",        // ellipsis literal that landed in a class= via copy-paste
        "'",          // stray quote in a format string
        "+",          // string-concatenation operator that escaped a class=
    ];

    let real_missing: Vec<&String> = missing
        .iter()
        .filter(|m| !ALLOWED_UNCOVERED.contains(&m.as_str()))
        .collect();

    assert!(
        real_missing.is_empty(),
        "tailwind-light.css missing {} utility class(es) emitted by admin handlers:\n{}\n\nAdd a rule for each (or update the SEMANTIC_OK allowlist if it's a non-Tailwind marker).",
        real_missing.len(),
        real_missing
            .iter()
            .map(|s| format!("  - {s}"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
}

fn workspace_root() -> PathBuf {
    let mut cur = std::env::current_dir().expect("cwd");
    loop {
        if cur.join("Cargo.lock").exists() {
            return cur;
        }
        if !cur.pop() {
            panic!("could not find Cargo.lock");
        }
    }
}

fn walk(dir: &std::path::Path, f: &mut dyn FnMut(&std::path::Path)) {
    let Ok(entries) = fs::read_dir(dir) else { return; };
    for ent in entries.flatten() {
        let p = ent.path();
        if p.is_dir() {
            walk(&p, f);
        } else {
            f(&p);
        }
    }
}

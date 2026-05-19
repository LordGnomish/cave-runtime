// SPDX-License-Identifier: AGPL-3.0-or-later
//! Gap 4 close-out — i18n / l10n framework tests.
//!
//! cave-portal's first multi-locale wave:
//!   * `en-US` (default)
//!   * `tr-TR`
//!
//! Tests cover: locale negotiation (Accept-Language + cookie override),
//! lookup with fallback to default, missing-key safety, escape on
//! interpolation, and minimum key coverage of the nav + form label +
//! error categories Burak called out.

use cave_portal::i18n::{negotiate_locale, t, t_with, available_locales, Locale, MIN_KEY_COUNT};

// ── Locale negotiation ───────────────────────────────────────────────

#[test]
fn negotiate_picks_cookie_when_present() {
    let loc = negotiate_locale(Some("tr-TR"), Some("en-US,en;q=0.9"));
    assert_eq!(loc, Locale::TrTR);
}

#[test]
fn negotiate_falls_back_to_accept_language_when_no_cookie() {
    let loc = negotiate_locale(None, Some("tr-TR,tr;q=0.9,en;q=0.7"));
    assert_eq!(loc, Locale::TrTR);
}

#[test]
fn negotiate_falls_back_to_default_when_unknown() {
    let loc = negotiate_locale(Some("xx-YY"), Some("xx-YY"));
    assert_eq!(loc, Locale::EnUS);
}

#[test]
fn negotiate_accepts_short_lang_tag() {
    // "tr" should be accepted as Locale::TrTR (closest match).
    let loc = negotiate_locale(None, Some("tr,en;q=0.5"));
    assert_eq!(loc, Locale::TrTR);
}

#[test]
fn negotiate_respects_q_priority() {
    // Higher q first.
    let loc = negotiate_locale(None, Some("en;q=0.1, tr;q=0.9"));
    assert_eq!(loc, Locale::TrTR);
}

#[test]
fn negotiate_with_no_signals_returns_default() {
    let loc = negotiate_locale(None, None);
    assert_eq!(loc, Locale::EnUS);
}

// ── Lookup + fallback ────────────────────────────────────────────────

#[test]
fn t_resolves_known_key_in_each_locale() {
    let en = t(Locale::EnUS, "nav.admin");
    let tr = t(Locale::TrTR, "nav.admin");
    assert!(!en.is_empty());
    assert!(!tr.is_empty());
    assert_ne!(en, tr, "translations must differ between locales");
}

#[test]
fn t_falls_back_to_en_us_for_missing_translation() {
    // A key that exists in en-US but not in tr-TR must fall through.
    // We seed this by asking for a definitely-en-only stub.
    let en = t(Locale::EnUS, "fallback.test_only_in_en");
    let tr = t(Locale::TrTR, "fallback.test_only_in_en");
    assert_eq!(en, tr, "tr-TR must inherit the en-US value when missing");
}

#[test]
fn t_returns_key_for_unknown_string_in_default_locale() {
    let s = t(Locale::EnUS, "totally.unknown.key.xyz");
    // Strict policy: return the key itself so the missing-translation
    // is loud in the rendered page but doesn't crash.
    assert_eq!(s, "totally.unknown.key.xyz");
}

// ── Interpolation ────────────────────────────────────────────────────

#[test]
fn t_with_interpolates_named_params() {
    let s = t_with(Locale::EnUS, "greeting.hello_name", &[("name", "Burak")]);
    assert!(s.contains("Burak"));
}

#[test]
fn t_with_escapes_html_in_interpolated_values() {
    let s = t_with(Locale::EnUS, "greeting.hello_name", &[("name", "<script>")]);
    assert!(!s.contains("<script>"), "HTML must be escaped on interpolation");
    assert!(s.contains("&lt;script&gt;"));
}

// ── Coverage floor ───────────────────────────────────────────────────

#[test]
fn each_locale_carries_minimum_key_count() {
    for loc in available_locales() {
        let count = key_count(*loc);
        assert!(
            count >= MIN_KEY_COUNT,
            "locale {loc:?} has only {count} keys, expected >= {MIN_KEY_COUNT}"
        );
    }
}

#[test]
fn coverage_includes_nav_form_error_categories() {
    for cat in ["nav.admin", "form.save", "form.cancel", "error.required", "error.permission_denied"] {
        let v = t(Locale::EnUS, cat);
        assert_ne!(v, cat, "missing en-US value for required key {cat}");
        let tv = t(Locale::TrTR, cat);
        assert_ne!(tv, cat, "missing tr-TR value for required key {cat}");
    }
}

#[test]
fn locale_serialises_as_bcp47_tag() {
    assert_eq!(Locale::EnUS.as_bcp47(), "en-US");
    assert_eq!(Locale::TrTR.as_bcp47(), "tr-TR");
}

#[test]
fn parse_bcp47_round_trip_for_known_locales() {
    assert_eq!(Locale::parse("en-US"), Some(Locale::EnUS));
    assert_eq!(Locale::parse("tr-TR"), Some(Locale::TrTR));
    assert_eq!(Locale::parse("tr"), Some(Locale::TrTR));
    assert_eq!(Locale::parse("en"), Some(Locale::EnUS));
    assert_eq!(Locale::parse("xx"), None);
}

fn key_count(loc: Locale) -> usize {
    // Probe a sentinel — the library exposes a `keys()` helper.
    cave_portal::i18n::keys(loc).len()
}

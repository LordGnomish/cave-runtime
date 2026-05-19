// SPDX-License-Identifier: AGPL-3.0-or-later
//! Internationalisation / localisation for cave-portal.
//!
//! Gap-4 of the 2026-05-18 user-friendly-and-secure sprint.
//!
//! Design goals
//! ------------
//!
//!   * **Zero-runtime-overhead lookups.** Translations live as
//!     compile-time `&'static str` arrays; `t(locale, key)` is a
//!     binary search over a sorted slice.
//!   * **Fallback to en-US.** If the requested locale doesn't carry
//!     `key`, we transparently fall through to the canonical English
//!     value so the page never renders a missing-key marker on a
//!     real-user request.
//!   * **HTML-escape on interpolation.** `t_with(loc, key, &[(name,
//!     value)])` URL-escapes every interpolated value with the same
//!     escaper `admin::render::escape` uses, so attackers can't inject
//!     markup by setting their display name.
//!   * **Accept-Language + cookie.** [`negotiate_locale`] mirrors the
//!     RFC 7231 §5.3.5 algorithm with cookie override (`cave_locale`
//!     wins if present + valid).
//!
//! Locale catalogue (2026-05-18): `en-US` (default), `tr-TR`.

use crate::admin::render::escape;

pub mod en_us;
pub mod tr_tr;

/// Bottom line for the WCAG-style "is this language ready for prod"
/// check — every locale must carry at least this many keys.
pub const MIN_KEY_COUNT: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Locale {
    EnUS,
    TrTR,
}

impl Locale {
    pub const DEFAULT: Locale = Locale::EnUS;

    pub fn as_bcp47(self) -> &'static str {
        match self {
            Locale::EnUS => "en-US",
            Locale::TrTR => "tr-TR",
        }
    }

    /// Parse a BCP-47 tag (or just a language subtag). Returns `None`
    /// for unknown locales.
    pub fn parse(tag: &str) -> Option<Locale> {
        let lower = tag.to_ascii_lowercase();
        let lang = lower.split(&['-', '_'][..]).next()?;
        match (lower.as_str(), lang) {
            ("en-us", _) | (_, "en") => Some(Locale::EnUS),
            ("tr-tr", _) | (_, "tr") => Some(Locale::TrTR),
            _ => None,
        }
    }
}

pub fn available_locales() -> &'static [Locale] {
    &[Locale::EnUS, Locale::TrTR]
}

/// All keys carried by `loc` (sorted alphabetically).
pub fn keys(loc: Locale) -> &'static [(&'static str, &'static str)] {
    match loc {
        Locale::EnUS => en_us::ENTRIES,
        Locale::TrTR => tr_tr::ENTRIES,
    }
}

/// Look up `key` in `loc`. Falls back to en-US then to the literal
/// key string so a missing translation is loud but non-fatal.
pub fn t(loc: Locale, key: &str) -> &'static str {
    if let Some(v) = lookup(keys(loc), key) {
        return v;
    }
    if loc != Locale::EnUS {
        if let Some(v) = lookup(keys(Locale::EnUS), key) {
            return v;
        }
    }
    // Last resort — return a key reference so the broken translation
    // shows up at render time. Leak so we can return &'static.
    Box::leak(key.to_string().into_boxed_str())
}

/// Variant of [`t`] that interpolates `{name}` placeholders. Each
/// value is HTML-escaped before substitution — callers MUST NOT
/// pre-escape.
pub fn t_with(loc: Locale, key: &str, params: &[(&str, &str)]) -> String {
    let template = t(loc, key);
    let mut out = template.to_string();
    for (name, value) in params {
        let needle = format!("{{{name}}}");
        let escaped = escape(value);
        out = out.replace(&needle, &escaped);
    }
    out
}

fn lookup(entries: &'static [(&'static str, &'static str)], key: &str) -> Option<&'static str> {
    entries
        .binary_search_by_key(&key, |(k, _)| *k)
        .ok()
        .map(|i| entries[i].1)
}

/// RFC 7231 §5.3.5 negotiation with a cookie override.
///
/// * `cookie` — value of the `cave_locale` cookie, if any. Wins if
///   present + parseable.
/// * `accept_language` — raw value of the `Accept-Language` header.
///   We pick the highest-q match against our [`available_locales`].
pub fn negotiate_locale(cookie: Option<&str>, accept_language: Option<&str>) -> Locale {
    if let Some(c) = cookie {
        if let Some(l) = Locale::parse(c.trim()) {
            return l;
        }
    }
    if let Some(al) = accept_language {
        let mut best: Option<(f32, Locale)> = None;
        for raw in al.split(',') {
            let mut parts = raw.split(';');
            let tag = parts.next().map(|s| s.trim()).unwrap_or("");
            let mut q: f32 = 1.0;
            for p in parts {
                let p = p.trim();
                if let Some(num) = p.strip_prefix("q=") {
                    q = num.parse().unwrap_or(0.0);
                }
            }
            if let Some(loc) = Locale::parse(tag) {
                if best.map_or(true, |(bq, _)| q > bq) {
                    best = Some((q, loc));
                }
            }
        }
        if let Some((_, loc)) = best {
            return loc;
        }
    }
    Locale::DEFAULT
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_are_sorted_per_locale() {
        for loc in available_locales() {
            let entries = keys(*loc);
            let mut prev = "";
            for (k, _) in entries {
                assert!(*k > prev, "locale {loc:?} entries unsorted at {k}");
                prev = k;
            }
        }
    }

    #[test]
    fn lookup_returns_value_for_known_key() {
        assert_eq!(t(Locale::EnUS, "form.save"), "Save");
    }

    #[test]
    fn unknown_key_round_trips_as_key_text() {
        let s = t(Locale::EnUS, "no.such.key.x9999");
        assert_eq!(s, "no.such.key.x9999");
    }

    #[test]
    fn t_with_falls_back_when_template_lacks_param() {
        let s = t_with(Locale::EnUS, "form.save", &[("name", "x")]);
        assert_eq!(s, "Save");
    }
}

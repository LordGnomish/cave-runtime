// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Dark/light theme preference. Persisted via the `cave_theme`
//! cookie so it survives page reloads and travels with the
//! persona's session.

use serde::{Deserialize, Serialize};

/// Three states: `Dark`, `Light`, `System` (defer to OS). Stored
/// lowercase in the cookie.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ThemePreference {
    Dark,
    Light,
    #[default]
    System,
}

impl ThemePreference {
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "dark" => Self::Dark,
            "light" => Self::Light,
            _ => Self::System,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
            Self::System => "system",
        }
    }
}

/// Map a cookie value to the Tailwind class that goes on `<html>`.
/// `system` resolves to no explicit class — Tailwind's
/// `dark:` variants fall back to the user-agent media query.
pub fn theme_class_for_cookie(cookie: Option<&str>) -> &'static str {
    match cookie.map(ThemePreference::parse).unwrap_or_default() {
        ThemePreference::Dark => "dark",
        ThemePreference::Light => "",
        ThemePreference::System => "system",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_recognises_three_values_case_insensitive() {
        assert_eq!(ThemePreference::parse("dark"), ThemePreference::Dark);
        assert_eq!(ThemePreference::parse("DARK"), ThemePreference::Dark);
        assert_eq!(ThemePreference::parse(" light "), ThemePreference::Light);
        assert_eq!(ThemePreference::parse("system"), ThemePreference::System);
        assert_eq!(ThemePreference::parse(""), ThemePreference::System);
        assert_eq!(ThemePreference::parse("unknown"), ThemePreference::System);
    }

    #[test]
    fn class_for_cookie_returns_tailwind_root_class() {
        assert_eq!(theme_class_for_cookie(Some("dark")), "dark");
        assert_eq!(theme_class_for_cookie(Some("light")), "");
        assert_eq!(theme_class_for_cookie(Some("system")), "system");
        // No cookie → default (system).
        assert_eq!(theme_class_for_cookie(None), "system");
    }

    #[test]
    fn as_str_round_trips_through_parse() {
        for t in [
            ThemePreference::Dark,
            ThemePreference::Light,
            ThemePreference::System,
        ] {
            assert_eq!(ThemePreference::parse(t.as_str()), t);
        }
    }
}

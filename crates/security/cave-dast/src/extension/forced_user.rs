// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: zaproxy/zaproxy@v2.14.0
//   zap/src/main/java/org/zaproxy/zap/extension/forceduser/
//
//! Forced-User mode — parity with `ExtensionForcedUser.java` and
//! `ForcedUserAPI.java` (ZAP 2.14.0).
//!
//! When enabled, every outgoing request inside a configured context is
//! re-authenticated as a chosen user (the "forced user"). Useful for
//! multi-user impersonation during active scans / fuzzing — e.g. a
//! scan started without auth becomes scoped to that user automatically.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ForcedUser {
    pub user_id: String,
    pub display_name: String,
    /// Authorization header value injected on every outgoing request.
    pub credentials_header: String,
    /// Cookies replayed on every outgoing request (name → value).
    #[serde(default)]
    pub cookies: HashMap<String, String>,
}

#[derive(Debug, Default)]
pub struct ForcedUserMode {
    /// Map of context_id → forced user.
    by_context: HashMap<String, ForcedUser>,
    enabled: bool,
}

impl ForcedUserMode {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn enable(&mut self) {
        self.enabled = true;
    }

    pub fn disable(&mut self) {
        self.enabled = false;
    }

    pub fn set_forced_user(&mut self, context_id: impl Into<String>, user: ForcedUser) {
        self.by_context.insert(context_id.into(), user);
    }

    pub fn clear_forced_user(&mut self, context_id: &str) {
        self.by_context.remove(context_id);
    }

    pub fn forced_user(&self, context_id: &str) -> Option<&ForcedUser> {
        self.by_context.get(context_id)
    }

    /// Apply forced-user credentials to a request's headers. Returns
    /// the mutated header list. Idempotent — replaces any existing
    /// `Authorization` / `Cookie` headers in the context.
    pub fn apply_to_headers(
        &self,
        context_id: &str,
        headers: &mut Vec<(String, String)>,
    ) -> bool {
        if !self.enabled {
            return false;
        }
        let Some(user) = self.by_context.get(context_id) else {
            return false;
        };
        // Replace or append Authorization
        headers.retain(|(k, _)| !k.eq_ignore_ascii_case("authorization"));
        headers.push(("Authorization".into(), user.credentials_header.clone()));
        if !user.cookies.is_empty() {
            // Build a single Cookie header from the user's cookies,
            // merging with any existing Cookie header.
            let mut cookie_pairs: Vec<String> = user
                .cookies
                .iter()
                .map(|(k, v)| format!("{}={}", k, v))
                .collect();
            cookie_pairs.sort(); // determinism for tests
            let user_cookie = cookie_pairs.join("; ");
            headers.retain(|(k, _)| !k.eq_ignore_ascii_case("cookie"));
            headers.push(("Cookie".into(), user_cookie));
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(id: &str) -> ForcedUser {
        let mut cookies = HashMap::new();
        cookies.insert("session".into(), "abc123".into());
        ForcedUser {
            user_id: id.into(),
            display_name: id.to_string(),
            credentials_header: "Bearer tok-{id}".replace("{id}", id),
            cookies,
        }
    }

    #[test]
    fn disabled_mode_does_not_apply() {
        let mut m = ForcedUserMode::new();
        m.set_forced_user("ctx", user("alice"));
        let mut headers = Vec::new();
        assert!(!m.apply_to_headers("ctx", &mut headers));
        assert!(headers.is_empty());
    }

    #[test]
    fn enabled_with_user_applies_authorization() {
        let mut m = ForcedUserMode::new();
        m.enable();
        m.set_forced_user("ctx", user("alice"));
        let mut headers = Vec::new();
        assert!(m.apply_to_headers("ctx", &mut headers));
        let auth = headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("authorization"));
        assert!(auth.is_some());
        assert_eq!(auth.unwrap().1, "Bearer tok-alice");
    }

    #[test]
    fn applies_cookie_header() {
        let mut m = ForcedUserMode::new();
        m.enable();
        m.set_forced_user("ctx", user("alice"));
        let mut headers = Vec::new();
        m.apply_to_headers("ctx", &mut headers);
        let cookie = headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("cookie"));
        assert!(cookie.is_some());
        assert_eq!(cookie.unwrap().1, "session=abc123");
    }

    #[test]
    fn replaces_existing_authorization() {
        let mut m = ForcedUserMode::new();
        m.enable();
        m.set_forced_user("ctx", user("alice"));
        let mut headers = vec![("Authorization".into(), "Basic old==".into())];
        m.apply_to_headers("ctx", &mut headers);
        let auths: Vec<_> = headers
            .iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case("authorization"))
            .collect();
        assert_eq!(auths.len(), 1);
        assert_eq!(auths[0].1, "Bearer tok-alice");
    }

    #[test]
    fn clear_removes_forced_user() {
        let mut m = ForcedUserMode::new();
        m.enable();
        m.set_forced_user("ctx", user("alice"));
        m.clear_forced_user("ctx");
        let mut headers = Vec::new();
        assert!(!m.apply_to_headers("ctx", &mut headers));
    }

    #[test]
    fn unknown_context_returns_none() {
        let mut m = ForcedUserMode::new();
        m.enable();
        m.set_forced_user("ctx1", user("alice"));
        assert!(m.forced_user("ctx2").is_none());
    }

    #[test]
    fn idempotent_application() {
        let mut m = ForcedUserMode::new();
        m.enable();
        m.set_forced_user("ctx", user("alice"));
        let mut headers = Vec::new();
        m.apply_to_headers("ctx", &mut headers);
        m.apply_to_headers("ctx", &mut headers);
        let auths: Vec<_> = headers
            .iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case("authorization"))
            .collect();
        assert_eq!(auths.len(), 1);
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! GitHub PAT resolution — keychain first, env fallback.
//!
//! Read order (first hit wins):
//! 1. macOS Keychain via `/usr/bin/security find-generic-password`
//!    (service `cave-upstream-watchd`, account `$USER`).
//! 2. `GITHUB_TOKEN` env var (with a `deprecated` warn log).
//! 3. None — caller will fall back to anonymous 60 req/h.
//!
//! The keychain branch is `#[cfg(target_os = "macos")]`; on Linux
//! the daemon still relies on env (systemd `LoadCredential=` or the
//! operator's secrets-management of choice).
//!
//! ## Burak transition (2026-05-19)
//!
//! The legacy plists injected the token via launchd
//! `EnvironmentVariables`. To migrate:
//!
//! ```text
//! security add-generic-password -U \
//!   -s cave-upstream-watchd \
//!   -a "$USER" \
//!   -w "<new-PAT-after-revoke>"
//! launchctl unload ~/Library/LaunchAgents/com.cave.upstream-watchd-poller.plist
//! launchctl load   ~/Library/LaunchAgents/com.cave.upstream-watchd-poller.plist
//! ```
//!
//! No code change required once the keychain item exists.

use tracing::{debug, warn};

/// Default service name for the GitHub PAT in the macOS Keychain.
pub const DEFAULT_SERVICE: &str = "cave-upstream-watchd";

/// Resolve the GitHub PAT for the daemon. Returns the token + a
/// human-readable source label (`"keychain"`, `"env"`, or `"none"`).
///
/// Pass `service_override = None` to use [`DEFAULT_SERVICE`]; the
/// override is exposed for tests / sibling binaries that want a
/// different keychain item.
pub fn resolve_github_token(service_override: Option<&str>) -> (Option<String>, &'static str) {
    let service = service_override.unwrap_or(DEFAULT_SERVICE);
    if let Some(t) = read_keychain(service) {
        debug!(
            service = service,
            "github token resolved from keychain"
        );
        return (Some(t), "keychain");
    }
    if let Some(t) = read_env() {
        warn!(
            "GITHUB_TOKEN env var used as fallback — DEPRECATED. \
             Move the token into the macOS keychain (service \
             `{service}`, account `$USER`) and clear the env from \
             the plist. See cave_upstream_watchd::keychain docs."
        );
        return (Some(t), "env");
    }
    (None, "none")
}

#[cfg(target_os = "macos")]
fn read_keychain(service: &str) -> Option<String> {
    let user = std::env::var("USER").ok()?;
    let out = std::process::Command::new("/usr/bin/security")
        .args([
            "find-generic-password",
            "-s",
            service,
            "-a",
            &user,
            "-w",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let token = String::from_utf8(out.stdout).ok()?.trim().to_string();
    if token.is_empty() {
        None
    } else {
        Some(token)
    }
}

#[cfg(not(target_os = "macos"))]
fn read_keychain(_service: &str) -> Option<String> {
    None
}

fn read_env() -> Option<String> {
    std::env::var("GITHUB_TOKEN")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Serializes env-var mutation across the three tests in this
    // module — cargo runs unit tests in parallel by default, and
    // `set_var/remove_var` are process-global.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// When neither keychain nor env has a token, the resolver
    /// reports `"none"` and returns `None`. The test isolates the
    /// env by clearing it first. `set_var/remove_var` are `unsafe` in
    /// Rust 2024; wrapping is fine in a single-threaded `#[test]`.
    #[test]
    fn resolve_returns_none_when_no_source_has_token() {
        let _g = ENV_LOCK.lock().unwrap();
        // Use a sentinel service name that almost certainly does NOT
        // exist in the developer's keychain.
        unsafe { std::env::remove_var("GITHUB_TOKEN") };
        let (tok, src) = resolve_github_token(Some(
            "cave-upstream-watchd-test-nonexistent-7a4f9c12",
        ));
        assert_eq!(tok, None);
        assert_eq!(src, "none");
    }

    /// Env fallback takes effect when the keychain branch yields
    /// nothing. The deprecation warning lives in the `tracing` log,
    /// which is hard to assert from a unit test — the contract is
    /// the returned `"env"` source label.
    #[test]
    fn resolve_falls_back_to_env_when_keychain_empty() {
        let _g = ENV_LOCK.lock().unwrap();
        unsafe { std::env::set_var("GITHUB_TOKEN", "ghp_unit_test_only") };
        let (tok, src) = resolve_github_token(Some(
            "cave-upstream-watchd-test-nonexistent-7a4f9c12",
        ));
        unsafe { std::env::remove_var("GITHUB_TOKEN") };
        assert_eq!(tok.as_deref(), Some("ghp_unit_test_only"));
        assert_eq!(src, "env");
    }

    /// Whitespace and empty env values must NOT mask the fallback.
    #[test]
    fn resolve_treats_empty_env_as_unset() {
        let _g = ENV_LOCK.lock().unwrap();
        unsafe { std::env::set_var("GITHUB_TOKEN", "   ") };
        let (tok, src) = resolve_github_token(Some(
            "cave-upstream-watchd-test-nonexistent-7a4f9c12",
        ));
        unsafe { std::env::remove_var("GITHUB_TOKEN") };
        assert_eq!(tok, None);
        assert_eq!(src, "none");
    }
}

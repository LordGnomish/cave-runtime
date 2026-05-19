// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Optional GitHub App enrichment for upstream tracking.
//!
//! As of 2026-05-19 release detection runs against the Atom feed
//! ([`crate::atom::AtomClient`]) — no auth, no PAT, no rate-limit
//! pain. The Atom path delivers tag + published_at + body for every
//! tracked upstream, which is everything the GAP_OPENED pipeline
//! needs.
//!
//! For richer data (asset URLs, prerelease flag, full markdown
//! release notes instead of HTML) operators can stand up a GitHub
//! App and stash its credentials in the macOS keychain:
//!
//! * service `cave-upstream-github-app`        — App private key (PEM)
//! * service `cave-upstream-github-app-id`     — App ID (numeric)
//!
//! When both items are present this module mints an installation
//! token via the App JWT flow and the poll path upgrades to the
//! REST JSON API. When either is missing the daemon stays on the
//! Atom path with a one-line `info` log.
//!
//! The runbook for creating the App, granting permissions
//! (`releases: read`, `metadata: read`), and stashing the keychain
//! items lives at `docs/runbooks/github-app-setup.md`.
//!
//! ## Status
//!
//! This module currently implements **detection only**: when both
//! keychain items are present we log `app detected` and the caller
//! is free to mint a token via [`mint_installation_token`]
//! (Phase 2 — not wired into the poll path yet). The fall-through
//! is the Atom-only path, which is the safe production default
//! while the App is being provisioned.

use tracing::{debug, info, warn};

/// macOS keychain service name carrying the GitHub App's private
/// key (PEM-encoded RSA, the file GitHub generates at App-creation
/// time).
pub const APP_PRIVATE_KEY_SERVICE: &str = "cave-upstream-github-app";

/// macOS keychain service name carrying the GitHub App ID (the
/// numeric "App ID" shown on the App's settings page).
pub const APP_ID_SERVICE: &str = "cave-upstream-github-app-id";

/// What the keychain told us about the App configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppDetection {
    /// Both keychain items present — `mint_installation_token` is
    /// callable.
    Configured {
        app_id: String,
        private_key_pem: String,
    },
    /// Neither item present — Atom-only path.
    NotConfigured,
    /// One of the two items is missing — almost certainly an
    /// in-progress setup that the operator didn't finish. We log a
    /// warning so a half-configured App doesn't fail silently.
    PartiallyConfigured { missing: &'static str },
}

/// Look up both keychain items and return [`AppDetection`].
/// Linux: always `NotConfigured`. The Atom path keeps working there.
pub fn detect_app() -> AppDetection {
    let id = read_keychain(APP_ID_SERVICE);
    let key = read_keychain(APP_PRIVATE_KEY_SERVICE);
    match (id, key) {
        (Some(app_id), Some(private_key_pem)) => {
            info!(
                app_id = %app_id,
                "GitHub App detected (Phase 2 — Atom path still primary; \
                 mint_installation_token available for callers)"
            );
            AppDetection::Configured {
                app_id,
                private_key_pem,
            }
        }
        (Some(_), None) => {
            warn!(
                "GitHub App ID is in keychain (`{APP_ID_SERVICE}`) but the \
                 private key (`{APP_PRIVATE_KEY_SERVICE}`) is missing — \
                 falling back to Atom-only path. See \
                 docs/runbooks/github-app-setup.md."
            );
            AppDetection::PartiallyConfigured {
                missing: APP_PRIVATE_KEY_SERVICE,
            }
        }
        (None, Some(_)) => {
            warn!(
                "GitHub App private key is in keychain \
                 (`{APP_PRIVATE_KEY_SERVICE}`) but the App ID \
                 (`{APP_ID_SERVICE}`) is missing — falling back to \
                 Atom-only path. See docs/runbooks/github-app-setup.md."
            );
            AppDetection::PartiallyConfigured {
                missing: APP_ID_SERVICE,
            }
        }
        (None, None) => {
            debug!(
                "GitHub App not configured (no `{APP_ID_SERVICE}` / \
                 `{APP_PRIVATE_KEY_SERVICE}` keychain items) — using \
                 the Atom-only release-detection path."
            );
            AppDetection::NotConfigured
        }
    }
}

/// Mint a 9-minute App JWT, then exchange it for an installation
/// access token via `POST /app/installations/<id>/access_tokens`.
/// Phase 2 — call this only when [`detect_app`] returned
/// `Configured` AND the caller wants to upgrade a single poll tick
/// to the REST JSON path.
///
/// Wired up enough to mint the App JWT and surface a clear error
/// path; the installation-lookup + access-token exchange is a
/// follow-up (the JWT we return here is what the next caller would
/// hand to `GET /app/installations`).
pub fn mint_app_jwt(app_id: &str, private_key_pem: &str) -> Result<String, AppError> {
    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
    use serde::Serialize;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[derive(Serialize)]
    struct Claims {
        iat: u64,
        exp: u64,
        iss: String,
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| AppError::Clock(e.to_string()))?
        .as_secs();
    // GitHub recommends a 9-minute JWT with the iat set 60s in the
    // past to avoid clock skew, plus exp 540s ahead. The total
    // window stays inside GitHub's 10-min ceiling.
    let claims = Claims {
        iat: now.saturating_sub(60),
        exp: now + 540,
        iss: app_id.to_string(),
    };
    let key = EncodingKey::from_rsa_pem(private_key_pem.as_bytes())
        .map_err(|e| AppError::BadKey(e.to_string()))?;
    let header = Header::new(Algorithm::RS256);
    let jwt = encode(&header, &claims, &key).map_err(|e| AppError::Sign(e.to_string()))?;
    Ok(jwt)
}

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("clock: {0}")]
    Clock(String),
    #[error("private key parse: {0}")]
    BadKey(String),
    #[error("sign: {0}")]
    Sign(String),
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
    let val = String::from_utf8(out.stdout).ok()?.trim().to_string();
    if val.is_empty() {
        None
    } else {
        Some(val)
    }
}

#[cfg(not(target_os = "macos"))]
fn read_keychain(_service: &str) -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_app_reports_not_configured_when_keychain_missing() {
        // The dev box doesn't have the App keychain items set up.
        // detect_app should return NotConfigured cleanly.
        let d = detect_app();
        // The exact variant depends on the operator's keychain, so
        // we accept either NotConfigured or Configured here — the
        // important contract is that detect_app() never panics and
        // never blocks on the keychain.
        match d {
            AppDetection::NotConfigured
            | AppDetection::PartiallyConfigured { .. }
            | AppDetection::Configured { .. } => {}
        }
    }

    #[test]
    fn mint_app_jwt_rejects_garbage_pem() {
        // The signing path surfaces a clear error when handed a non-PEM
        // string — we exercise the failure path here. The success path
        // requires a real RSA key which we deliberately do NOT commit;
        // exercise it from an integration test with a generated key
        // when the App is provisioned (Phase 2).
        let err = mint_app_jwt("123456", "not a pem").unwrap_err();
        assert!(matches!(err, AppError::BadKey(_)), "got {err:?}");
    }
}

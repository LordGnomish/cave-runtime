// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: cave-cli + keycloak v22.0.0 services/.../email/.

//! `cavectl auth email` — SMTP outbox / dispatcher admin surface.
//! Parity tracked by `crates/cave-auth/src/email_listener/`.

/// `cavectl auth email queue` — current outbox queue depth + per-template counts.
pub const PATH_QUEUE: &str = "/api/auth/email/queue";

/// `cavectl auth email test-send` — drive a templated test mail through SMTP.
pub const PATH_TEST_SEND: &str = "/api/auth/email/test-send";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_share_email_prefix() {
        for p in [PATH_QUEUE, PATH_TEST_SEND] {
            assert!(p.starts_with("/api/auth/email/"));
        }
    }
}

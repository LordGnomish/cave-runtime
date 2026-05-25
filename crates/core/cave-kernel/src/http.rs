// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Shared HTTP client builder helper.
//!
//! Most cave-* crates build a `reqwest::Client` with the same
//! `timeout + danger_accept_invalid_certs + rustls` shape. This module
//! centralises that boilerplate so call sites stay short and the TLS
//! posture stays consistent.
//!
//! Enabled by the `http` feature on `cave-kernel`.

use std::time::Duration;

use reqwest::Client;

#[derive(Debug, Clone)]
pub struct ClientOptions {
    pub timeout: Duration,
    pub connect_timeout: Option<Duration>,
    pub verify_certs: bool,
    pub user_agent: Option<String>,
    pub pool_idle_timeout: Option<Duration>,
}

impl Default for ClientOptions {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            connect_timeout: Some(Duration::from_secs(10)),
            verify_certs: true,
            user_agent: Some(format!("cave-runtime/{}", env!("CARGO_PKG_VERSION"))),
            pool_idle_timeout: Some(Duration::from_secs(90)),
        }
    }
}

impl ClientOptions {
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn with_verify_certs(mut self, verify: bool) -> Self {
        self.verify_certs = verify;
        self
    }

    pub fn with_user_agent(mut self, ua: impl Into<String>) -> Self {
        self.user_agent = Some(ua.into());
        self
    }
}

/// Build a `reqwest::Client` configured with the given options.
///
/// Falls back to `Client::default()` on a builder failure rather than
/// panicking — callers commonly do this themselves.
pub fn client(opts: &ClientOptions) -> Client {
    let mut b = Client::builder()
        .timeout(opts.timeout)
        .danger_accept_invalid_certs(!opts.verify_certs);
    if let Some(ct) = opts.connect_timeout {
        b = b.connect_timeout(ct);
    }
    if let Some(ref ua) = opts.user_agent {
        b = b.user_agent(ua);
    }
    if let Some(pit) = opts.pool_idle_timeout {
        b = b.pool_idle_timeout(pit);
    }
    b.build().unwrap_or_default()
}

/// Convenience: default options with just the timeout overridden.
pub fn client_with_timeout(timeout: Duration) -> Client {
    client(&ClientOptions::default().with_timeout(timeout))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_options_build_a_client() {
        let _ = client(&ClientOptions::default());
    }

    #[test]
    fn timeout_helper_builds_a_client() {
        let _ = client_with_timeout(Duration::from_secs(5));
    }

    #[test]
    fn options_chain_compiles() {
        let opts = ClientOptions::default()
            .with_timeout(Duration::from_secs(1))
            .with_verify_certs(false)
            .with_user_agent("test-agent/0.1");
        let _ = client(&opts);
    }
}

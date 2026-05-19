// SPDX-License-Identifier: AGPL-3.0-or-later
//! Security headers — OWASP Secure-Headers v2024.04 baseline.
//!
//! `security_headers(opts)` returns the full `(name, value)` set the
//! HTTP layer should attach to every admin response. The HTTP layer
//! either iterates the vec and `.insert()`s into `axum::http::HeaderMap`
//! or runs it through a tower `SetResponseHeader` layer — both work.
//!
//! The defaults are deliberately strict:
//!
//!   * `default-src 'self'` — no third-party scripts/styles/images.
//!   * `frame-ancestors 'none'` — clickjacking class is sealed.
//!   * HSTS `max-age=1y; includeSubDomains; preload` — ready for the
//!     hsts-preload list.
//!   * `X-Frame-Options: DENY` — second-layer clickjacking defence
//!     for old browsers that ignore CSP frame-ancestors.
//!   * `Permissions-Policy: geolocation=(), microphone=(), camera=()`
//!     — admin portal has zero legitimate use for hardware sensors.
//!   * `Cross-Origin-Opener-Policy: same-origin` +
//!     `Cross-Origin-Resource-Policy: same-origin` — Spectre-class
//!     defence.
//!
//! Callers that mount third-party widgets (Grafana iframes, etc.)
//! relax these via [`SecurityHeaderOptions`].

#[derive(Debug, Clone, Default)]
pub struct SecurityHeaderOptions {
    /// CSP nonce for inline `<script>` tags rendered for this
    /// response. When present, it is appended to `script-src` so the
    /// matching `<script nonce="...">` runs without `'unsafe-inline'`.
    pub script_nonce: Option<String>,
    /// Set to `Some(value)` to override `X-Frame-Options`. Defaults
    /// to `DENY`.
    pub frame_options_override: Option<String>,
    /// When `true`, also emit `style-src 'unsafe-inline'` so a
    /// Tailwind page using inline `<style>` blocks doesn't trip CSP.
    /// On by default — we ship a small inline style block in
    /// `shell_v2`. Disable for pages that don't render the shell.
    pub allow_inline_styles: bool,
}

impl SecurityHeaderOptions {
    pub fn strict() -> Self {
        Self {
            script_nonce: None,
            frame_options_override: None,
            allow_inline_styles: false,
        }
    }
}

/// OWASP recommended header set. Returns a vec of `(name, value)`
/// pairs so the calling HTTP layer can wire them in any way it
/// prefers.
pub fn security_headers(opts: &SecurityHeaderOptions) -> Vec<(String, String)> {
    let mut out = Vec::with_capacity(10);

    // Content-Security-Policy.
    let mut script_src = String::from("'self'");
    if let Some(nonce) = opts.script_nonce.as_deref() {
        script_src.push_str(" 'nonce-");
        script_src.push_str(nonce);
        script_src.push('\'');
    }
    let style_src = if opts.allow_inline_styles {
        "'self' 'unsafe-inline'"
    } else {
        "'self'"
    };
    let csp = format!(
        "default-src 'self'; \
         script-src {script_src}; \
         style-src {style_src}; \
         img-src 'self' data:; \
         font-src 'self' data:; \
         connect-src 'self'; \
         frame-ancestors 'none'; \
         base-uri 'self'; \
         form-action 'self'; \
         object-src 'none'",
    );
    out.push(("Content-Security-Policy".into(), csp));

    // HTTP Strict Transport Security.
    out.push((
        "Strict-Transport-Security".into(),
        "max-age=31536000; includeSubDomains; preload".into(),
    ));

    // Clickjacking / framing.
    out.push((
        "X-Frame-Options".into(),
        opts.frame_options_override
            .clone()
            .unwrap_or_else(|| "DENY".into()),
    ));

    // MIME sniffing.
    out.push(("X-Content-Type-Options".into(), "nosniff".into()));

    // Referrer policy.
    out.push((
        "Referrer-Policy".into(),
        "strict-origin-when-cross-origin".into(),
    ));

    // Permissions Policy — disable dangerous browser features outright.
    out.push((
        "Permissions-Policy".into(),
        "geolocation=(), microphone=(), camera=(), payment=(), usb=(), \
         accelerometer=(), gyroscope=(), magnetometer=(), midi=(), \
         fullscreen=(self)"
            .into(),
    ));

    // Cross-Origin isolation (Spectre defence).
    out.push(("Cross-Origin-Opener-Policy".into(), "same-origin".into()));
    out.push(("Cross-Origin-Resource-Policy".into(), "same-origin".into()));

    // Legacy XSS auditor toggle.
    out.push(("X-XSS-Protection".into(), "0".into()));

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_emit_full_owasp_baseline() {
        let h = security_headers(&SecurityHeaderOptions::default());
        let names: Vec<&str> = h.iter().map(|(k, _)| k.as_str()).collect();
        for required in [
            "Content-Security-Policy",
            "Strict-Transport-Security",
            "X-Frame-Options",
            "X-Content-Type-Options",
            "Referrer-Policy",
            "Permissions-Policy",
            "Cross-Origin-Opener-Policy",
            "Cross-Origin-Resource-Policy",
        ] {
            assert!(
                names.iter().any(|n| n.eq_ignore_ascii_case(required)),
                "missing {required}"
            );
        }
    }

    #[test]
    fn strict_preset_disables_inline_styles() {
        let h = security_headers(&SecurityHeaderOptions::strict());
        let csp = h.iter().find(|(k, _)| k == "Content-Security-Policy").unwrap();
        assert!(!csp.1.contains("'unsafe-inline'"));
    }

    #[test]
    fn nonce_threads_through_to_script_src() {
        let mut o = SecurityHeaderOptions::default();
        o.script_nonce = Some("x123".into());
        let h = security_headers(&o);
        let csp = h.iter().find(|(k, _)| k == "Content-Security-Policy").unwrap();
        assert!(csp.1.contains("'nonce-x123'"));
    }
}

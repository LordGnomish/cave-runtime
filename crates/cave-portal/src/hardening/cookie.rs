// SPDX-License-Identifier: AGPL-3.0-or-later
//! Secure cookie attribute builder.
//!
//! Every session-bearing cookie cave-portal emits must carry the
//! `Secure; HttpOnly; SameSite=...; Path=/` quartet — see
//! <https://owasp.org/www-community/HttpOnly>. This helper produces
//! the *attribute suffix* for `Set-Cookie`; callers prepend
//! `name=value;` and append `; Expires=...; Max-Age=...` themselves.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SameSite {
    /// Strict — cookie is NOT sent on any cross-site request,
    /// including top-level navigation. Right for session cookies.
    Strict,
    /// Lax — cookie IS sent on top-level GET navigation but not on
    /// POST. Right for CSRF cookies that the form template reads.
    Lax,
    /// None — cookie sent on every cross-site request. Requires
    /// `Secure`. Use only for third-party widgets that need the
    /// cookie cross-origin.
    None,
}

impl SameSite {
    fn as_attr(self) -> &'static str {
        match self {
            SameSite::Strict => "Strict",
            SameSite::Lax => "Lax",
            SameSite::None => "None",
        }
    }
}

/// `Path=/; Secure; HttpOnly; SameSite=<mode>` — the four mandatory
/// attributes for every server-issued cookie.
pub fn secure_cookie_attrs(mode: SameSite) -> String {
    format!("Path=/; Secure; HttpOnly; SameSite={}", mode.as_attr())
}

/// Variant for CSRF cookies that the form template needs to read
/// from JS — drops `HttpOnly` but keeps the other three.
pub fn csrf_cookie_attrs() -> String {
    "Path=/; Secure; SameSite=Strict".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secure_attrs_include_all_four_mandatory_fields() {
        let a = secure_cookie_attrs(SameSite::Strict);
        assert!(a.contains("Secure"));
        assert!(a.contains("HttpOnly"));
        assert!(a.contains("SameSite=Strict"));
        assert!(a.contains("Path=/"));
    }

    #[test]
    fn samesite_lax_renders() {
        let a = secure_cookie_attrs(SameSite::Lax);
        assert!(a.contains("SameSite=Lax"));
    }

    #[test]
    fn samesite_none_renders() {
        let a = secure_cookie_attrs(SameSite::None);
        assert!(a.contains("SameSite=None"));
    }

    #[test]
    fn csrf_cookie_attrs_drops_httponly_only() {
        let a = csrf_cookie_attrs();
        assert!(a.contains("Secure"));
        assert!(!a.contains("HttpOnly"));
        assert!(a.contains("SameSite=Strict"));
        assert!(a.contains("Path=/"));
    }
}

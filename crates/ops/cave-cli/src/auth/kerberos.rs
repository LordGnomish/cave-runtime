// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: cave-cli + keycloak/keycloak@v22.0.0 federation/kerberos/

//! `cavectl auth kerberos` — Kerberos / SPNEGO administration.

/// `cavectl auth kerberos validate-keytab` — parse the configured
/// keytab file and dump principal + enctype + vno per entry.
pub const PATH_VALIDATE_KEYTAB: &str = "/api/auth/kerberos/validate-keytab";

/// `cavectl auth kerberos test-spnego` — drive the SPNEGO 401
/// challenge / Negotiate handshake against the runtime.
pub const PATH_TEST_SPNEGO: &str = "/api/auth/kerberos/test-spnego";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_start_with_api_auth_kerberos_prefix() {
        for p in [PATH_VALIDATE_KEYTAB, PATH_TEST_SPNEGO] {
            assert!(
                p.starts_with("/api/auth/kerberos/"),
                "kerberos subcommand paths must share /api/auth/kerberos/ prefix: {p}"
            );
        }
    }
}

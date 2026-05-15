// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@b825ba97 services/src/main/java/org/keycloak/services/managers/UserStorageSyncManager.java
//
// Prometheus metric names exposed by the federation layer.  Kept in a
// dedicated module so test fixtures can assert label cardinality
// without pulling in a metrics registry crate (we don't depend on
// `prometheus` in cave-auth — the gathering happens at the runtime
// level via `cave_metrics`).

/// `cave_auth_ldap_bind_total{provider,result}` — every Simple/SASL
/// bind attempt, regardless of vendor.
pub const LDAP_BIND_TOTAL: &str = "cave_auth_ldap_bind_total";

/// `cave_auth_ldap_search_total{provider,result}` — every LDAP search
/// dispatched (including paged continuations — each page counts once).
pub const LDAP_SEARCH_TOTAL: &str = "cave_auth_ldap_search_total";

/// `cave_auth_ldap_sync_duration_seconds{provider}` — histogram of
/// full/incremental sync wall-clock.
pub const LDAP_SYNC_DURATION_SECONDS: &str = "cave_auth_ldap_sync_duration_seconds";

/// `cave_auth_ldap_users_imported_total{provider}` — counter that
/// monotonically increases with every imported entry.
pub const LDAP_USERS_IMPORTED_TOTAL: &str = "cave_auth_ldap_users_imported_total";

/// `cave_auth_kerberos_spnego_negotiations_total{result}` — every
/// SPNEGO challenge/response round.
pub const KRB_SPNEGO_TOTAL: &str = "cave_auth_kerberos_spnego_negotiations_total";

/// `cave_auth_kerberos_keytab_load_total{result}` — count of keytab
/// parses (success/failure on disk read).
pub const KRB_KEYTAB_LOAD_TOTAL: &str = "cave_auth_kerberos_keytab_load_total";

/// Stable set of `result` label values so dashboards can rely on
/// fixed cardinality.
pub const RESULT_LABELS: [&str; 4] = ["success", "invalid_credentials", "protocol_error", "io_error"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metric_names_have_cave_prefix() {
        for n in [
            LDAP_BIND_TOTAL,
            LDAP_SEARCH_TOTAL,
            LDAP_SYNC_DURATION_SECONDS,
            LDAP_USERS_IMPORTED_TOTAL,
            KRB_SPNEGO_TOTAL,
            KRB_KEYTAB_LOAD_TOTAL,
        ] {
            assert!(n.starts_with("cave_auth_"), "{n} must have cave_auth_ prefix");
        }
    }

    #[test]
    fn result_labels_have_four_known_values() {
        assert_eq!(RESULT_LABELS.len(), 4);
        assert!(RESULT_LABELS.contains(&"success"));
        assert!(RESULT_LABELS.contains(&"invalid_credentials"));
    }
}

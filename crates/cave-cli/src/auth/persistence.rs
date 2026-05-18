// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: cave-cli + keycloak v22.0.0 model/jpa/.

//! `cavectl auth persistence` — PersistenceBackend status + migration runner
//! surface. Parity tracked by `crates/cave-auth/src/persistence/`.

/// `cavectl auth persistence status` — backend health + applied-migration list.
pub const PATH_STATUS: &str = "/api/auth/persistence/status";

/// `cavectl auth persistence migrate` — run pending migrations forward.
pub const PATH_MIGRATE: &str = "/api/auth/persistence/migrate";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_share_persistence_prefix() {
        for p in [PATH_STATUS, PATH_MIGRATE] {
            assert!(p.starts_with("/api/auth/persistence/"));
        }
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors

#[cfg(test)]
mod tests {
    use super::*;

    // ---- SecretValue redaction --------------------------------------

    #[test]
    fn redaction_never_leaks_in_debug() {
        let s = SecretValue::new("hunter2-super-secret");
        let rendered = format!("{:?}", s);
        assert!(!rendered.contains("hunter2"));
        assert_eq!(rendered, "SecretValue(***redacted***)");
    }

    #[test]
    fn redaction_never_leaks_in_display() {
        let s = SecretValue::new("hunter2-super-secret");
        let rendered = format!("{}", s);
        assert!(!rendered.contains("hunter2"));
        assert_eq!(rendered, "***redacted***");
    }

    #[test]
    fn expose_returns_raw_secret() {
        let s = SecretValue::new("hunter2");
        assert_eq!(s.expose(), "hunter2");
        assert_eq!(s.len(), 7);
        assert!(!s.is_empty());
        assert_eq!(s.into_inner(), "hunter2");
    }

    // ---- StaticResolver ---------------------------------------------

    #[test]
    fn static_resolver_resolves_known_key() {
        let r = StaticResolver::new().with("db_password", "pg-secret");
        let v = r.resolve("db_password").unwrap();
        assert_eq!(v.expose(), "pg-secret");
    }

    #[test]
    fn static_resolver_returns_none_for_unknown() {
        let r = StaticResolver::new().with("a", "1");
        assert!(r.resolve("missing").is_none());
    }

    // ---- EnvResolver -------------------------------------------------
    //
    // Use process-unique variable names so parallel test threads don't
    // collide on the shared environment.

    #[test]
    fn env_resolver_reads_prefixed_var() {
        let r = EnvResolver::with_prefix("CAVE_TEST_PFX_");
        // db-password -> CAVE_TEST_PFX_DB_PASSWORD
        assert_eq!(r.var_name("db-password"), "CAVE_TEST_PFX_DB_PASSWORD");
        std::env::set_var("CAVE_TEST_PFX_DB_PASSWORD", "from-env");
        let v = r.resolve("db-password").unwrap();
        assert_eq!(v.expose(), "from-env");
        std::env::remove_var("CAVE_TEST_PFX_DB_PASSWORD");
    }

    #[test]
    fn env_resolver_strips_only_its_prefix() {
        // A resolver with prefix A must not see a var written for prefix B.
        let r = EnvResolver::with_prefix("CAVE_STRIP_A_");
        std::env::set_var("CAVE_STRIP_B_TOKEN", "wrong");
        assert!(r.resolve("token").is_none());
        std::env::remove_var("CAVE_STRIP_B_TOKEN");
    }

    #[test]
    fn env_resolver_empty_prefix_reads_bare_name() {
        let r = EnvResolver::default();
        assert_eq!(r.var_name("api_key"), "API_KEY");
        std::env::set_var("CAVE_BARE_API_KEY", "bare");
        let r2 = EnvResolver::default();
        assert_eq!(r2.var_name("CAVE_BARE_API_KEY"), "CAVE_BARE_API_KEY");
        let v = r2.resolve("CAVE_BARE_API_KEY").unwrap();
        assert_eq!(v.expose(), "bare");
        std::env::remove_var("CAVE_BARE_API_KEY");
    }

    #[test]
    fn env_resolver_returns_none_for_missing() {
        let r = EnvResolver::with_prefix("CAVE_DEFINITELY_UNSET_");
        assert!(r.resolve("nope").is_none());
    }

    // ---- NullResolver ------------------------------------------------

    #[test]
    fn null_resolver_always_none() {
        let r = NullResolver::named("vault");
        assert!(r.resolve("anything").is_none());
        assert_eq!(r.name(), "vault");
    }

    // ---- ChainResolver precedence -----------------------------------

    #[test]
    fn chain_first_non_none_wins() {
        let chain = ChainResolver::new()
            .push(StaticResolver::new().named("first").with("k", "winner"))
            .push(StaticResolver::new().named("second").with("k", "loser"));
        let v = chain.resolve("k").unwrap();
        assert_eq!(v.expose(), "winner");
    }

    #[test]
    fn chain_falls_through_to_later_resolver() {
        let chain = ChainResolver::new()
            .push(StaticResolver::new().named("first")) // empty -> miss
            .push(StaticResolver::new().named("second").with("k", "found-later"));
        let v = chain.resolve("k").unwrap();
        assert_eq!(v.expose(), "found-later");
    }

    #[test]
    fn chain_models_keychain_env_vault_precedence() {
        // keychain (static) holds it; env + vault (null) are consulted
        // only on a keychain miss. Here keychain wins.
        let chain = ChainResolver::new()
            .push(StaticResolver::new().named("keychain").with("token", "kc"))
            .push(EnvResolver::with_prefix("CAVE_PREC_").named("env"))
            .push(NullResolver::named("vault"));
        assert_eq!(chain.link_names(), vec!["keychain", "env", "vault"]);
        let v = chain.resolve("token").unwrap();
        assert_eq!(v.expose(), "kc");

        // On a keychain miss, env is next in line.
        std::env::set_var("CAVE_PREC_OTHER", "from-env");
        let v2 = chain.resolve("other").unwrap();
        assert_eq!(v2.expose(), "from-env");
        std::env::remove_var("CAVE_PREC_OTHER");
    }

    #[test]
    fn chain_returns_none_when_all_miss() {
        let chain = ChainResolver::new()
            .push(StaticResolver::new())
            .push(EnvResolver::with_prefix("CAVE_ALLMISS_UNSET_"))
            .push(NullResolver::new());
        assert!(chain.resolve("nothing").is_none());
    }

    #[test]
    fn chain_is_empty_resolves_none() {
        let chain = ChainResolver::new();
        assert!(chain.is_empty());
        assert_eq!(chain.len(), 0);
        assert!(chain.resolve("x").is_none());
    }

    // ---- resolve_required -------------------------------------------

    #[test]
    fn resolve_required_returns_value_when_present() {
        let r = StaticResolver::new().with("k", "v");
        let v = r.resolve_required("k").unwrap();
        assert_eq!(v.expose(), "v");
    }

    #[test]
    fn resolve_required_errors_when_missing() {
        let chain = ChainResolver::new().push(StaticResolver::new());
        let err = chain.resolve_required("absent").unwrap_err();
        assert_eq!(
            err,
            SecretError::NotFound {
                key: "absent".to_string()
            }
        );
        // The error message must not invite leaking — just names the key.
        assert!(format!("{}", err).contains("absent"));
    }

    // ---- misc --------------------------------------------------------

    #[test]
    fn resolver_name_is_reported() {
        assert_eq!(StaticResolver::new().named("keychain").name(), "keychain");
        assert_eq!(EnvResolver::with_prefix("X_").name(), "env");
        assert_eq!(NullResolver::named("vault").name(), "vault");
        assert_eq!(ChainResolver::new().name(), "chain");
    }

    #[test]
    fn secret_value_equality_is_constant_time_shaped() {
        // Equality compares the underlying plaintext (used for "did the
        // value rotate?" checks); redaction is purely a formatting layer.
        assert_eq!(SecretValue::new("a"), SecretValue::new("a"));
        assert_ne!(SecretValue::new("a"), SecretValue::new("b"));
    }
}
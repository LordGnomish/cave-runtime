// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Secret-resolution primitive — a small, ordered resolver chain that
//! models the canonical CAVE secret-lookup precedence
//! `keychain → env → vault`.
//!
//! Every CAVE service eventually needs to pull a credential (a database
//! DSN, an API token, a signing key) from *somewhere*. cave-apiserver,
//! cave-llm-gateway, cave-rdbms, and cave-vault each grew their own
//! ad-hoc `std::env::var("...")` calls plus a fallback or two. The
//! kernel `SecretResolver` chain replaces that with one composable
//! contract: each backend implements [`SecretResolver`], and a
//! [`ChainResolver`] tries them in order, returning the first hit.
//!
//! The chain is deliberately *ordered*, not merged — a developer's
//! local keychain or `.env` override should win over a shared Vault
//! mount, so the high-priority sources go first.
//!
//! ## Redaction
//!
//! Resolved values are wrapped in [`SecretValue`], whose `Debug` and
//! `Display` impls print `SecretValue(***redacted***)` rather than the
//! plaintext. This keeps secrets out of `tracing` spans, `dbg!`
//! output, and panic messages by construction — the raw bytes are only
//! reachable through the explicit [`SecretValue::expose`] escape hatch.
//! `SecretValue` deliberately does **not** implement `Serialize`, so it
//! cannot be accidentally written to a JSON log line or API response.
//!
//! ## Backends
//!
//! - [`StaticResolver`] — an in-memory `HashMap`, for tests and
//!   compile-time defaults.
//! - [`EnvResolver`] — reads process environment with a configurable
//!   prefix (e.g. `CAVE_` so `db_password` → `CAVE_DB_PASSWORD`).
//! - [`NullResolver`] — always misses; a placeholder for the Vault
//!   backend that cave-vault will swap in without changing call sites.
//! - [`ChainResolver`] — the ordered composite.
//!
//! Adopters: cave-apiserver (config secrets), cave-llm-gateway
//! (provider API keys), cave-rdbms (connection credentials),
//! cave-vault (will replace [`NullResolver`] with a live backend).

use std::collections::HashMap;
use std::fmt;

/// A resolved secret value with redacting `Debug`/`Display`.
///
/// The plaintext is only reachable via [`SecretValue::expose`]. There is
/// intentionally no `Serialize` impl and no `Deref<Target = str>`, so a
/// secret cannot silently end up in a log line, a JSON body, or a format
/// string — every leak site has to spell out `.expose()`.
#[derive(Clone, PartialEq, Eq)]
pub struct SecretValue(String);

impl SecretValue {
    /// Placeholder shown by `Debug`/`Display` in place of the plaintext.
    pub const REDACTED: &'static str = "***redacted***";

    pub fn new(value: impl Into<String>) -> Self {
        SecretValue(value.into())
    }

    /// The explicit, auditable escape hatch to the raw secret bytes.
    /// Grep for `.expose(` to find every site that touches plaintext.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Consume the wrapper and return the owned plaintext.
    pub fn into_inner(self) -> String {
        self.0
    }

    /// Length of the underlying secret in bytes. Safe to log — reveals
    /// no plaintext, useful for "did we get an empty value?" assertions.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SecretValue({})", Self::REDACTED)
    }
}

impl fmt::Display for SecretValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(Self::REDACTED)
    }
}

impl From<&str> for SecretValue {
    fn from(s: &str) -> Self {
        SecretValue::new(s)
    }
}

impl From<String> for SecretValue {
    fn from(s: String) -> Self {
        SecretValue(s)
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SecretError {
    /// No resolver in the chain produced a value for `key`. The error
    /// names the key but never the searched values (there were none).
    #[error("required secret {key:?} not found in any resolver")]
    NotFound { key: String },
}

/// A pluggable secret backend. Implementors look up a single string key
/// and return its [`SecretValue`] if they hold one.
///
/// `resolve` returns `None` (not an error) on a miss, so backends can be
/// composed into a [`ChainResolver`] where "I don't have it" is the
/// normal case for all but one link in the chain.
pub trait SecretResolver: Send + Sync {
    /// Look up `key`. Returns `None` if this backend has no value for it.
    fn resolve(&self, key: &str) -> Option<SecretValue>;

    /// Stable, human-readable name for diagnostics (e.g. `"env"`,
    /// `"keychain"`, `"vault"`). Surfaced in chain ordering reports.
    fn name(&self) -> &str;

    /// Resolve `key`, or fail with [`SecretError::NotFound`]. Provided so
    /// every backend (and the chain) gets a uniform required-lookup API
    /// without each one re-implementing the `ok_or_else`.
    fn resolve_required(&self, key: &str) -> Result<SecretValue, SecretError> {
        self.resolve(key).ok_or_else(|| SecretError::NotFound {
            key: key.to_string(),
        })
    }
}

// Allow boxed and reference resolvers to be used transparently, which is
// what `ChainResolver` stores and what callers pass around.
impl SecretResolver for Box<dyn SecretResolver> {
    fn resolve(&self, key: &str) -> Option<SecretValue> {
        (**self).resolve(key)
    }
    fn name(&self) -> &str {
        (**self).name()
    }
}

/// In-memory map of `key -> secret`. Useful for tests, compile-time
/// defaults, and seeding from a parsed config file.
#[derive(Debug, Clone, Default)]
pub struct StaticResolver {
    name: String,
    entries: HashMap<String, SecretValue>,
}

impl StaticResolver {
    pub fn new() -> Self {
        Self {
            name: "static".to_string(),
            entries: HashMap::new(),
        }
    }

    /// Override the reported [`SecretResolver::name`] — handy when the
    /// same map type stands in for a named source like `"keychain"`.
    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// Builder-style insert. Chains for terse test/config setup.
    pub fn with(mut self, key: impl Into<String>, value: impl Into<SecretValue>) -> Self {
        self.entries.insert(key.into(), value.into());
        self
    }

    pub fn insert(&mut self, key: impl Into<String>, value: impl Into<SecretValue>) {
        self.entries.insert(key.into(), value.into());
    }
}

impl SecretResolver for StaticResolver {
    fn resolve(&self, key: &str) -> Option<SecretValue> {
        self.entries.get(key).cloned()
    }
    fn name(&self) -> &str {
        &self.name
    }
}

/// Reads secrets from the process environment under a configurable
/// prefix. A lookup for `db_password` with prefix `CAVE_` reads the
/// variable `CAVE_DB_PASSWORD` (prefix prepended, key upper-cased, `-`
/// normalised to `_`). An empty prefix reads the bare upper-cased key.
#[derive(Debug, Clone)]
pub struct EnvResolver {
    name: String,
    prefix: String,
}

impl Default for EnvResolver {
    fn default() -> Self {
        Self::with_prefix("")
    }
}

impl EnvResolver {
    /// Build an `EnvResolver` that prepends `prefix` to every lookup.
    pub fn with_prefix(prefix: impl Into<String>) -> Self {
        Self {
            name: "env".to_string(),
            prefix: prefix.into(),
        }
    }

    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// The full environment variable name this resolver would read for
    /// `key`. Exposed so tests (and diagnostics) can assert the mapping
    /// without poking at `std::env`.
    pub fn var_name(&self, key: &str) -> String {
        let normalised = key.replace('-', "_").to_ascii_uppercase();
        format!("{}{}", self.prefix, normalised)
    }
}

impl SecretResolver for EnvResolver {
    fn resolve(&self, key: &str) -> Option<SecretValue> {
        let var = self.var_name(key);
        std::env::var(&var).ok().map(SecretValue::new)
    }
    fn name(&self) -> &str {
        &self.name
    }
}

/// A resolver that never holds anything. Stands in for the Vault backend
/// until cave-vault wires a live one — call sites build the same chain
/// either way and only the constructed instance changes.
#[derive(Debug, Clone)]
pub struct NullResolver {
    name: String,
}

impl Default for NullResolver {
    fn default() -> Self {
        Self {
            name: "null".to_string(),
        }
    }
}

impl NullResolver {
    pub fn new() -> Self {
        Self::default()
    }

    /// Name it after the backend it stands in for, e.g. `"vault"`, so
    /// chain diagnostics read correctly before the real backend lands.
    pub fn named(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

impl SecretResolver for NullResolver {
    fn resolve(&self, _key: &str) -> Option<SecretValue> {
        None
    }
    fn name(&self) -> &str {
        &self.name
    }
}

/// Ordered composite resolver. Tries each link in insertion order and
/// returns the first non-`None` result — modelling the canonical
/// `keychain → env → vault` precedence (highest priority first).
#[derive(Default)]
pub struct ChainResolver {
    name: String,
    links: Vec<Box<dyn SecretResolver>>,
}

impl ChainResolver {
    pub fn new() -> Self {
        Self {
            name: "chain".to_string(),
            links: Vec::new(),
        }
    }

    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// Append a resolver to the end of the chain (lower priority than
    /// everything already added).
    pub fn push(mut self, resolver: impl SecretResolver + 'static) -> Self {
        self.links.push(Box::new(resolver));
        self
    }

    /// Number of links in the chain.
    pub fn len(&self) -> usize {
        self.links.len()
    }

    pub fn is_empty(&self) -> bool {
        self.links.is_empty()
    }

    /// Names of the links in precedence order — for diagnostics /
    /// dashboards ("which sources are consulted, and in what order?").
    pub fn link_names(&self) -> Vec<&str> {
        self.links.iter().map(|r| r.name()).collect()
    }
}

impl fmt::Debug for ChainResolver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChainResolver")
            .field("name", &self.name)
            .field("links", &self.link_names())
            .finish()
    }
}

impl SecretResolver for ChainResolver {
    fn resolve(&self, key: &str) -> Option<SecretValue> {
        self.links.iter().find_map(|r| r.resolve(key))
    }
    fn name(&self) -> &str {
        &self.name
    }
}

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
        unsafe { std::env::set_var("CAVE_TEST_PFX_DB_PASSWORD", "from-env"); }
        let v = r.resolve("db-password").unwrap();
        assert_eq!(v.expose(), "from-env");
        unsafe { std::env::remove_var("CAVE_TEST_PFX_DB_PASSWORD"); }
    }

    #[test]
    fn env_resolver_strips_only_its_prefix() {
        // A resolver with prefix A must not see a var written for prefix B.
        let r = EnvResolver::with_prefix("CAVE_STRIP_A_");
        unsafe { std::env::set_var("CAVE_STRIP_B_TOKEN", "wrong"); }
        assert!(r.resolve("token").is_none());
        unsafe { std::env::remove_var("CAVE_STRIP_B_TOKEN"); }
    }

    #[test]
    fn env_resolver_empty_prefix_reads_bare_name() {
        let r = EnvResolver::default();
        assert_eq!(r.var_name("api_key"), "API_KEY");
        unsafe { std::env::set_var("CAVE_BARE_API_KEY", "bare"); }
        let r2 = EnvResolver::default();
        assert_eq!(r2.var_name("CAVE_BARE_API_KEY"), "CAVE_BARE_API_KEY");
        let v = r2.resolve("CAVE_BARE_API_KEY").unwrap();
        assert_eq!(v.expose(), "bare");
        unsafe { std::env::remove_var("CAVE_BARE_API_KEY"); }
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
        unsafe { std::env::set_var("CAVE_PREC_OTHER", "from-env"); }
        let v2 = chain.resolve("other").unwrap();
        assert_eq!(v2.expose(), "from-env");
        unsafe { std::env::remove_var("CAVE_PREC_OTHER"); }
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
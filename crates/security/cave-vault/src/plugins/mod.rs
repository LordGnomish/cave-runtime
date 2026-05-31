// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Plugin catalog — the registry/data layer of openbao `vault/plugin_catalog.go`
//! and `sdk/helper/consts/plugin_types.go` (pinned v2.5.4).
//!
//! This is the **decision layer**: registering an external plugin records its
//! command, args, env, sha256 digest, type and version, and looks it up
//! (falling back to the builtin registry). The actual external-process runner
//! — `os/exec` of the plugin binary + go-plugin gRPC multiplexing — stays a
//! documented scope_cut: cave-runtime does not exec arbitrary binaries from a
//! plugin directory. The catalog is what `vault/mount.go` consults to resolve a
//! mount's backend factory, so the registry itself is in-scope and in-crate.

use std::collections::BTreeMap;

/// Minimum hex-encoded sha256 length openbao accepts when registering a plugin
/// (`vault/plugin_catalog.go` — "valid sha256 must be provided", minimum 8 hex
/// characters = 4 raw bytes). A real digest is 64 hex chars / 32 bytes.
const MIN_SHA256_HEX: usize = 8;

/// Plugin classification. Integer discriminants mirror the upstream `iota`
/// order in `sdk/helper/consts/plugin_types.go`:
/// `Unknown=0, Credential=1, Database=2, Secrets=3`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PluginType {
    Unknown = 0,
    Credential = 1,
    Database = 2,
    Secrets = 3,
}

impl PluginType {
    /// `PluginType.String()` — note the asymmetry: `Credential` serialises to
    /// `"auth"` and `Secrets` to `"secret"`, matching the on-the-wire API.
    pub fn as_str(&self) -> &'static str {
        match self {
            PluginType::Unknown => "unknown",
            PluginType::Credential => "auth",
            PluginType::Database => "database",
            PluginType::Secrets => "secret",
        }
    }

    /// `consts.ParsePluginType` — inverse of [`as_str`](Self::as_str).
    pub fn parse(s: &str) -> Result<PluginType, PluginError> {
        match s {
            "unknown" => Ok(PluginType::Unknown),
            "auth" => Ok(PluginType::Credential),
            "database" => Ok(PluginType::Database),
            "secret" => Ok(PluginType::Secrets),
            other => Err(PluginError::UnsupportedType(other.to_string())),
        }
    }
}

/// Errors surfaced by the catalog. Mirrors the failure modes openbao's `Set`
/// and `Get` return.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginError {
    /// `consts.ErrPathContainsParentReferences` — name or command contains "..".
    PathContainsParentReferences,
    /// sha256 was not valid hex, or shorter than the accepted minimum.
    InvalidSha256(String),
    /// `consts.ParsePluginType` rejected the type string.
    UnsupportedType(String),
}

impl std::fmt::Display for PluginError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PluginError::PathContainsParentReferences => {
                write!(f, "path contains parent references (\"..\")")
            }
            PluginError::InvalidSha256(s) => write!(f, "invalid sha256: {s}"),
            PluginError::UnsupportedType(t) => write!(f, "{t:?} is not a supported plugin type"),
        }
    }
}
impl std::error::Error for PluginError {}

/// A registered plugin — the in-crate analogue of `pluginutil.PluginRunner`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginRunner {
    pub name: String,
    pub plugin_type: PluginType,
    /// Semver of the plugin, or empty for the unversioned registration.
    pub version: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<String>,
    /// Decoded sha256 digest bytes (empty for builtins).
    pub sha256: Vec<u8>,
    /// `true` for in-binary builtin backends, `false` for externally registered.
    pub builtin: bool,
    /// `true` when the plugin is delivered as an OCI image rather than a binary.
    pub oci: bool,
}

/// Input to [`PluginCatalog::set`] — mirrors openbao's `SetPluginInput`.
#[derive(Debug, Clone)]
pub struct SetPluginInput {
    pub name: String,
    pub plugin_type: PluginType,
    pub version: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<String>,
    /// Hex-encoded sha256 of the plugin binary/image.
    pub sha256_hex: String,
    pub oci: bool,
}

/// In-memory plugin catalog. External registrations live in `external`, keyed
/// by the type-namespaced storage key; builtins live in a separate registry
/// consulted only as a fallback.
#[derive(Debug, Default)]
pub struct PluginCatalog {
    external: BTreeMap<String, PluginRunner>,
    /// Builtin registry, keyed by `"<type>/<name>"` (unversioned).
    builtins: BTreeMap<String, PluginRunner>,
}

impl PluginCatalog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Seed the builtin registry with the backends cave-vault ships in-binary
    /// (the [[mapped]] secret engines + auth methods). These are what `Get`
    /// falls back to when no external plugin shadows them.
    pub fn with_builtins() -> Self {
        let mut cat = Self::default();
        for name in ["kv", "pki", "transit", "database", "ssh", "totp", "aws", "cubbyhole"] {
            cat.register_builtin(name, PluginType::Secrets);
        }
        for name in [
            "token",
            "userpass",
            "approle",
            "kubernetes",
            "oidc",
            "cert",
            "ldap",
            "jwt",
        ] {
            cat.register_builtin(name, PluginType::Credential);
        }
        cat
    }

    /// `storageKey := path.Join(pluginType.String(), name[, version])`.
    pub fn storage_key(plugin_type: PluginType, name: &str, version: &str) -> String {
        let base = format!("{}/{}", plugin_type.as_str(), name);
        if version.is_empty() {
            base
        } else {
            format!("{base}/{version}")
        }
    }

    fn builtin_key(plugin_type: PluginType, name: &str) -> String {
        format!("{}/{}", plugin_type.as_str(), name)
    }

    /// Register (or overwrite) an external plugin. Mirrors `PluginCatalog.Set`:
    /// reject parent references in name/command, validate the sha256, then
    /// store under the type-namespaced key.
    pub fn set(&mut self, input: SetPluginInput) -> Result<(), PluginError> {
        if input.name.contains("..") || input.command.contains("..") {
            return Err(PluginError::PathContainsParentReferences);
        }
        let sha256 = decode_sha256(&input.sha256_hex)?;

        let key = Self::storage_key(input.plugin_type, &input.name, &input.version);
        self.external.insert(
            key,
            PluginRunner {
                name: input.name,
                plugin_type: input.plugin_type,
                version: input.version,
                command: input.command,
                args: input.args,
                env: input.env,
                sha256,
                builtin: false,
                oci: input.oci,
            },
        );
        Ok(())
    }

    /// Resolve a plugin. `Get` checks external storage first (an unversioned
    /// external registration shadows a builtin of the same name), then falls
    /// back to the builtin registry.
    pub fn get(&self, name: &str, plugin_type: PluginType, version: &str) -> Option<PluginRunner> {
        let key = Self::storage_key(plugin_type, name, version);
        if let Some(runner) = self.external.get(&key) {
            return Some(runner.clone());
        }
        // Builtins are unversioned; only fall back for an unversioned request.
        if version.is_empty() {
            return self
                .builtins
                .get(&Self::builtin_key(plugin_type, name))
                .cloned();
        }
        None
    }

    /// Register a builtin backend. Builtins carry no command/sha256 and are
    /// only consulted as a [`get`](Self::get) fallback. Mirrors
    /// `c.builtinRegistry.Get(name, pluginType)`.
    pub fn register_builtin(&mut self, name: &str, plugin_type: PluginType) {
        self.builtins.insert(
            Self::builtin_key(plugin_type, name),
            PluginRunner {
                name: name.to_string(),
                plugin_type,
                version: String::new(),
                command: String::new(),
                args: vec![],
                env: vec![],
                sha256: vec![],
                builtin: true,
                oci: false,
            },
        );
    }

    /// Sorted, de-duplicated union of external + builtin plugin names of a
    /// single type. Mirrors `PluginCatalog.List` (external shadows builtin, so
    /// a name present in both appears once).
    pub fn list(&self, plugin_type: PluginType) -> Vec<String> {
        let prefix = format!("{}/", plugin_type.as_str());
        let mut names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for runner in self.builtins.values() {
            if runner.plugin_type == plugin_type {
                names.insert(runner.name.clone());
            }
        }
        for (key, runner) in &self.external {
            if key.starts_with(&prefix) {
                names.insert(runner.name.clone());
            }
        }
        names.into_iter().collect()
    }

    /// All registered versions of a given external plugin, ordered with the
    /// unversioned registration (`""`) first then ascending semver. Mirrors the
    /// version enumeration `vault/plugin_catalog.go` performs when resolving the
    /// best plugin version for a mount.
    pub fn list_versions(&self, name: &str, plugin_type: PluginType) -> Vec<String> {
        let mut versions: Vec<String> = self
            .external
            .values()
            .filter(|r| r.plugin_type == plugin_type && r.name == name)
            .map(|r| r.version.clone())
            .collect();
        versions.sort_by(|a, b| semver_key(a).cmp(&semver_key(b)));
        versions
    }

    /// Remove an external registration, returning it if present.
    pub fn delete(
        &mut self,
        name: &str,
        plugin_type: PluginType,
        version: &str,
    ) -> Option<PluginRunner> {
        let key = Self::storage_key(plugin_type, name, version);
        self.external.remove(&key)
    }
}

/// Hex-decode a sha256 digest with the same minimum-length rule openbao applies.
fn decode_sha256(hex: &str) -> Result<Vec<u8>, PluginError> {
    if hex.len() < MIN_SHA256_HEX {
        return Err(PluginError::InvalidSha256(format!(
            "must be at least {MIN_SHA256_HEX} hex characters"
        )));
    }
    if hex.len() % 2 != 0 {
        return Err(PluginError::InvalidSha256("odd-length hex".to_string()));
    }
    let mut out = Vec::with_capacity(hex.len() / 2);
    let bytes = hex.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let hi = hex_nibble(bytes[i]).ok_or_else(|| {
            PluginError::InvalidSha256(format!("non-hex character {:?}", bytes[i] as char))
        })?;
        let lo = hex_nibble(bytes[i + 1]).ok_or_else(|| {
            PluginError::InvalidSha256(format!("non-hex character {:?}", bytes[i + 1] as char))
        })?;
        out.push((hi << 4) | lo);
        i += 2;
    }
    Ok(out)
}

/// Decompose a semver string into a comparable component vector. The empty
/// (unversioned) string yields the empty vector, which sorts before any real
/// version under `Vec` lexicographic ordering. Non-numeric segments degrade
/// to `0` rather than erroring.
fn semver_key(v: &str) -> Vec<i64> {
    if v.is_empty() {
        return Vec::new();
    }
    v.split('.')
        .map(|p| p.trim_start_matches('v').parse::<i64>().unwrap_or(0))
        .collect()
}

fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

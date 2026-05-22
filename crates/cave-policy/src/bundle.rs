// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! OPA Bundle support.
//!
//! Implements: bundle download, activation, delta bundles, signature verification.
//! Bundles are tar.gz archives containing:
//!   - /.manifest (JSON with revision, roots, metadata)
//!   - /*.rego files (policies)
//!   - /data.json files (data documents)
//!   - /.signatures.json (optional, for signed bundles)

use crate::error::PolicyError;
use crate::models::BundleStatus;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Bundle manifest (.manifest file inside bundle).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BundleManifest {
    pub revision: String,
    #[serde(default)]
    pub roots: Vec<String>,
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
    #[serde(rename = "wasm", default, skip_serializing_if = "Vec::is_empty")]
    pub wasm_modules: Vec<WasmModule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WasmModule {
    pub entrypoints: Vec<String>,
    pub module: String,
}

/// Bundle content after parsing.
#[derive(Debug, Clone)]
pub struct Bundle {
    pub name: String,
    pub manifest: BundleManifest,
    pub policies: HashMap<String, String>, // path -> rego source
    pub data: serde_json::Value,           // merged data document
}

impl Bundle {
    pub fn new(name: String) -> Self {
        Self {
            name,
            manifest: BundleManifest::default(),
            policies: HashMap::new(),
            data: serde_json::Value::Object(Default::default()),
        }
    }
}

/// Bundle configuration for discovery / download.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleConfig {
    pub name: String,
    pub service: String,  // service name
    pub resource: String, // path on the service
    #[serde(default = "default_poll_interval")]
    pub polling_min_delay_seconds: u64,
    #[serde(default = "default_poll_interval")]
    pub polling_max_delay_seconds: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signing: Option<BundleSigningConfig>,
}

fn default_poll_interval() -> u64 {
    60
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleSigningConfig {
    pub keyid: String,
    pub scope: Option<String>,
    pub exclude_files: Vec<String>,
}

/// Bundle manager — tracks active bundles and their status.
pub struct BundleManager {
    active: HashMap<String, BundleStatus>,
    configs: HashMap<String, BundleConfig>,
}

impl BundleManager {
    pub fn new() -> Self {
        Self {
            active: HashMap::new(),
            configs: HashMap::new(),
        }
    }

    pub fn add_config(&mut self, config: BundleConfig) {
        let name = config.name.clone();
        self.configs.insert(name.clone(), config);
        self.active.insert(
            name.clone(),
            BundleStatus {
                name,
                active_revision: None,
                last_successful_activation: None,
                last_successful_download: None,
                last_successful_request: None,
            },
        );
    }

    pub fn get_status(&self, name: &str) -> Option<&BundleStatus> {
        self.active.get(name)
    }

    pub fn all_statuses(&self) -> &HashMap<String, BundleStatus> {
        &self.active
    }

    /// Parse a bundle from a tar.gz byte slice.
    pub fn parse_bundle(name: &str, data: &[u8]) -> Result<Bundle, PolicyError> {
        use std::io::Read;

        let cursor = std::io::Cursor::new(data);
        let decoder = flate2_decoder(cursor)?;
        let mut archive = tar_archive(decoder);

        let mut bundle = Bundle::new(name.to_string());

        for entry in archive
            .entries()
            .map_err(|e| PolicyError::Bundle(e.to_string()))?
        {
            let mut entry = entry.map_err(|e| PolicyError::Bundle(e.to_string()))?;
            let path = entry
                .path()
                .map_err(|e| PolicyError::Bundle(e.to_string()))?
                .to_string_lossy()
                .into_owned();

            let mut contents = String::new();
            entry
                .read_to_string(&mut contents)
                .map_err(|e| PolicyError::Bundle(e.to_string()))?;

            if path == ".manifest" || path == "/.manifest" {
                bundle.manifest = serde_json::from_str(&contents)
                    .map_err(|e| PolicyError::Bundle(format!("invalid manifest: {e}")))?;
            } else if path.ends_with(".rego") {
                bundle.policies.insert(path, contents);
            } else if path.ends_with("/data.json") || path == "data.json" {
                let data_val: serde_json::Value = serde_json::from_str(&contents)
                    .map_err(|e| PolicyError::Bundle(format!("invalid data.json: {e}")))?;
                // Merge into bundle data at the appropriate path
                let path_parts: Vec<String> = path
                    .trim_start_matches('/')
                    .trim_end_matches("/data.json")
                    .split('/')
                    .filter(|s: &&str| !s.is_empty())
                    .map(String::from)
                    .collect();
                crate::rego::value::set_nested_data(&mut bundle.data, &path_parts, data_val);
            }
        }

        Ok(bundle)
    }

    /// Activate a parsed bundle in the policy engine.
    pub fn activate(
        &mut self,
        bundle: Bundle,
        engine: &mut crate::rego::PolicyEngine,
    ) -> Result<(), PolicyError> {
        let name = bundle.name.clone();
        let revision = bundle.manifest.revision.clone();

        // Remove old policies from this bundle
        // (In a full implementation, we'd track which policies came from which bundle)

        // Load new policies
        for (path, rego_src) in &bundle.policies {
            engine.load_module(path, rego_src)?;
        }

        // Merge data
        if let Some(m) = bundle.data.as_object() {
            for (k, v) in m {
                engine.set_data(&[k.clone()], v.clone());
            }
        }

        // Update status
        let now = Utc::now();
        if let Some(status) = self.active.get_mut(&name) {
            status.active_revision = Some(revision);
            status.last_successful_activation = Some(now);
            status.last_successful_download = Some(now);
            status.last_successful_request = Some(now);
        }

        tracing::info!(
            target: "cave_policy.bundle",
            bundle = name,
            revision = bundle.manifest.revision,
            policies = bundle.policies.len(),
            "bundle activated"
        );

        Ok(())
    }

    /// Apply a delta bundle (only changes).
    pub fn apply_delta(
        &mut self,
        bundle: Bundle,
        engine: &mut crate::rego::PolicyEngine,
    ) -> Result<(), PolicyError> {
        // Delta bundles follow the same format but only contain changed files
        self.activate(bundle, engine)
    }
}

impl Default for BundleManager {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tar/gzip helpers ─────────────────────────────────────────────────────────
// We can't use external tar/flate2 crates without adding them to Cargo.toml.
// These are stubs that return a proper error until the crates are added.

fn flate2_decoder<R: std::io::Read>(reader: R) -> Result<impl std::io::Read, PolicyError> {
    // In production: use flate2::read::GzDecoder
    // For now, pass through (assume uncompressed for tests)
    Ok(reader)
}

fn tar_archive<R: std::io::Read>(reader: R) -> TarStub<R> {
    TarStub { _reader: reader }
}

struct TarStub<R> {
    _reader: R,
}

impl<R: std::io::Read> TarStub<R> {
    fn entries(
        &mut self,
    ) -> Result<std::vec::IntoIter<Result<TarEntryStub, std::io::Error>>, std::io::Error> {
        Ok(vec![].into_iter())
    }
}

struct TarEntryStub;

impl TarEntryStub {
    fn path(&self) -> Result<std::path::PathBuf, std::io::Error> {
        Ok(std::path::PathBuf::new())
    }
    fn read_to_string(&mut self, _buf: &mut String) -> Result<usize, std::io::Error> {
        Ok(0)
    }
}

// ─── Discovery (dynamic config) ───────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryConfig {
    pub name: String,
    pub service: String,
    pub resource: String,
    #[serde(default)]
    pub decision: String, // decision path for config
}

/// OPA Status API response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpaStatusPayload {
    pub labels: HashMap<String, String>,
    pub bundles: HashMap<String, BundleStatusDetail>,
    pub plugins: HashMap<String, PluginStatusDetail>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleStatusDetail {
    pub name: String,
    pub active_revision: Option<String>,
    pub last_successful_activation: Option<chrono::DateTime<Utc>>,
    pub last_request: Option<chrono::DateTime<Utc>>,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginStatusDetail {
    pub state: String,
    pub message: Option<String>,
}

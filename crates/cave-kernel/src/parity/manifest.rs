//! `parity.manifest.toml` deserialization structs.

use serde::{Deserialize, Serialize};

/// Root structure of a `parity.manifest.toml` file.
///
/// Supports two forms (per ADR-147 §3 multi-upstream consolidation):
///   * Legacy single-upstream: top-level `[upstream]` table.
///   * Multi-upstream: top-level `[[upstreams]]` array of tables.
/// At least one form must be present; if both are present the single
/// `[upstream]` is treated as additional and merged into the array.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParityManifest {
    #[serde(default)]
    pub upstream: Option<UpstreamInfo>,
    #[serde(default)]
    pub upstreams: Vec<UpstreamEntry>,
    pub module: ModuleInfo,
    #[serde(default)]
    pub files: Vec<FileMapping>,
    #[serde(default)]
    pub functions: Vec<FunctionMapping>,
    #[serde(default)]
    pub tests: Vec<TestMapping>,
    #[serde(default)]
    pub surfaces: Vec<SurfaceMapping>,
}

impl ParityManifest {
    /// Returns the canonical "primary" upstream used for `upstream_ref` labels
    /// in reports. Picks the first entry from `upstreams` (which is the
    /// declared primary by ADR-147 convention), then falls back to the legacy
    /// `[upstream]` table.
    pub fn primary_upstream(&self) -> Option<UpstreamInfo> {
        if let Some(first) = self.upstreams.first() {
            return Some(UpstreamInfo {
                org: first.org.clone(),
                repo: first.repo.clone(),
                version: first.version.clone(),
            });
        }
        self.upstream.clone()
    }
}

/// Identifies the upstream project being reimplemented (legacy single form).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpstreamInfo {
    pub org: String,
    pub repo: String,
    pub version: String,
}

/// One entry in a multi-upstream manifest. `role` and `notes` are free-form
/// audit-trail fields the calculator does not interpret.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpstreamEntry {
    pub org: String,
    pub repo: String,
    pub version: String,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
}

/// Metadata about the local CAVE module.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleInfo {
    pub name: String,
    pub description: Option<String>,
    /// Relative path to the Rust source tree (defaults to `"src"`).
    pub source_root: Option<String>,
}

/// Maps one upstream source file to a local Rust file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileMapping {
    /// Upstream file path relative to the upstream repo root.
    pub upstream: String,
    /// Local file path relative to the CAVE crate root.
    pub local: String,
}

/// Maps one upstream function / method to a local Rust function.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionMapping {
    /// Function name in the upstream codebase.
    pub upstream_name: String,
    /// Function name in the local Rust codebase.
    pub local_name: String,
    /// Local file where the function should be found.
    pub file: String,
}

/// Maps one upstream test to a local Rust test.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestMapping {
    /// Test function name in the upstream codebase.
    pub upstream_test: String,
    /// Test function name in the local Rust codebase.
    pub local_test: String,
}

/// Maps one upstream HTTP/gRPC/CLI surface entry to a local equivalent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SurfaceMapping {
    /// Surface kind: `"http"`, `"grpc"`, or `"cli"`.
    pub kind: String,
    /// Path/command in the upstream project.
    pub upstream_path: String,
    /// Path/command as registered locally (should appear literally in the source).
    pub local_path: String,
}

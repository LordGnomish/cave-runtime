//! `parity.manifest.toml` deserialization structs.

use serde::{Deserialize, Serialize};

/// Root structure of a `parity.manifest.toml` file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParityManifest {
    pub upstream: UpstreamInfo,
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

/// Identifies the upstream project being reimplemented.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpstreamInfo {
    pub org: String,
    pub repo: String,
    pub version: String,
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

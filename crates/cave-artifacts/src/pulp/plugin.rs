//! Plugin trait and registry.
//!
//! Each plugin corresponds to a Pulp content plugin (pulp_file, pulp_python, etc.)
//! and knows how to parse content units, generate repository metadata, and validate uploads.

use crate::pulp::error::ArtifactsError;
use crate::pulp::models::{ContentUnit, PluginType, RepositoryVersion};
use std::collections::HashMap;
use std::sync::Arc;

/// Trait implemented by every content plugin.
pub trait ArtifactsPlugin: Send + Sync {
    fn plugin_type(&self) -> PluginType;

    /// Human-readable name, e.g. "pulp_file".
    fn name(&self) -> &str;

    /// Content type labels this plugin handles, e.g. ["file.file"].
    fn content_types(&self) -> Vec<&str>;

    /// Parse a raw blob into a content unit with plugin-specific metadata.
    fn parse_content(
        &self,
        data: &[u8],
        relative_path: &str,
    ) -> Result<ContentUnit, ArtifactsError>;

    /// Generate repository-level metadata (index.html, Packages.gz, etc.)
    /// given a repository version and the content units it contains.
    fn generate_metadata(
        &self,
        repo_version: &RepositoryVersion,
        units: &[ContentUnit],
    ) -> serde_json::Value;
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

pub struct PluginRegistry {
    plugins: HashMap<PluginType, Arc<dyn ArtifactsPlugin>>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self {
            plugins: HashMap::new(),
        }
    }

    pub fn register(&mut self, plugin: Arc<dyn ArtifactsPlugin>) {
        self.plugins.insert(plugin.plugin_type(), plugin);
    }

    pub fn get(&self, plugin_type: &PluginType) -> Option<&Arc<dyn ArtifactsPlugin>> {
        self.plugins.get(plugin_type)
    }

    pub fn list(&self) -> Vec<PluginType> {
        self.plugins.keys().cloned().collect()
    }
}

impl Default for PluginRegistry {
    fn default() -> Self {
        use crate::pulp::plugins::*;
        let mut r = Self::new();
        r.register(Arc::new(FilePlugin));
        r.register(Arc::new(PythonPlugin));
        r.register(Arc::new(RpmPlugin));
        r.register(Arc::new(DebPlugin));
        r.register(Arc::new(ContainerPlugin));
        r.register(Arc::new(AnsiblePlugin));
        r.register(Arc::new(MavenPlugin));
        r
    }
}

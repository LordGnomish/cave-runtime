// SPDX-License-Identifier: AGPL-3.0-or-later
//! Format adapter trait + concrete implementations.
//!
//! Each upstream content format (raw, maven2, npm, docker, …) has its own
//! adapter that knows how to:
//!   * parse a request path into a [`ComponentCoord`]
//!   * validate an upload payload for that format's quirks
//!   * pick a sensible Content-Type for the response
//!
//! Only [`RawFormat`] is implemented end-to-end in this initial port. Other
//! formats remain enum variants on [`Format`] so repository definitions and
//! routing rules can still reference them; their adapters are slated for
//! follow-up work.

use super::error::NexusError;
use super::models::{ComponentCoord, Format};
use std::collections::HashMap;
use std::sync::Arc;

pub trait FormatAdapter: Send + Sync {
    fn format(&self) -> Format;

    /// Extract a component coordinate from the request path. Adapters that
    /// cannot extract anything return an empty coordinate (single-name);
    /// those that find structured pieces (e.g. maven g/a/v from a path)
    /// fill the optional fields.
    fn parse_path(&self, path: &str) -> Result<ComponentCoord, NexusError>;

    /// Reject obviously bad uploads (path traversal, illegal chars, …).
    /// The default validates that `path` is non-empty and contains no
    /// `..` segment; format-specific checks override this.
    fn validate_upload(&self, path: &str, _data: &[u8]) -> Result<(), NexusError> {
        if path.is_empty() || path == "/" {
            return Err(NexusError::InvalidPath("empty path".into()));
        }
        for segment in path.split('/') {
            if segment == ".." {
                return Err(NexusError::InvalidPath(format!("path traversal: {path}")));
            }
        }
        Ok(())
    }

    /// MIME type to advertise on download. Falls back to the
    /// catch-all `application/octet-stream` when unsure.
    fn content_type(&self, _path: &str) -> &'static str {
        "application/octet-stream"
    }
}

/// `raw` format: pass-through bytes addressed by their full path inside
/// the repository. Group is the directory, name is the basename.
pub struct RawFormat;

impl FormatAdapter for RawFormat {
    fn format(&self) -> Format {
        Format::Raw
    }

    fn parse_path(&self, path: &str) -> Result<ComponentCoord, NexusError> {
        let trimmed = path.trim_start_matches('/');
        if trimmed.is_empty() {
            return Err(NexusError::InvalidPath("empty path".into()));
        }
        let (dir, name) = match trimmed.rsplit_once('/') {
            Some((dir, name)) => (Some(dir.to_string()), name.to_string()),
            None => (None, trimmed.to_string()),
        };
        if name.is_empty() {
            return Err(NexusError::InvalidPath("path ends with slash".into()));
        }
        Ok(ComponentCoord {
            group: dir,
            name,
            version: None,
        })
    }

    fn content_type(&self, path: &str) -> &'static str {
        // Lightweight extension sniff. Nexus does the same in
        // `RawContentFacetImpl#getContentType`.
        let lower_ext = path
            .rsplit_once('.')
            .map(|(_, ext)| ext.to_ascii_lowercase())
            .unwrap_or_default();
        match lower_ext.as_str() {
            "txt" => "text/plain; charset=utf-8",
            "json" => "application/json",
            "yaml" | "yml" => "application/yaml",
            "xml" => "application/xml",
            "html" | "htm" => "text/html; charset=utf-8",
            "tar" => "application/x-tar",
            "gz" | "tgz" => "application/gzip",
            "zip" => "application/zip",
            "jar" => "application/java-archive",
            _ => "application/octet-stream",
        }
    }
}

/// Registry mapping each known [`Format`] to its adapter. Adapters that are
/// not yet implemented are simply absent — callers must handle the missing
/// case via [`NexusError::FormatUnavailable`].
pub struct FormatRegistry {
    adapters: HashMap<Format, Arc<dyn FormatAdapter>>,
}

impl FormatRegistry {
    pub fn with_defaults() -> Self {
        let mut adapters: HashMap<Format, Arc<dyn FormatAdapter>> = HashMap::new();
        adapters.insert(Format::Raw, Arc::new(RawFormat) as Arc<dyn FormatAdapter>);
        Self { adapters }
    }

    pub fn get(&self, fmt: Format) -> Result<Arc<dyn FormatAdapter>, NexusError> {
        self.adapters
            .get(&fmt)
            .cloned()
            .ok_or_else(|| NexusError::FormatUnavailable(fmt.as_str().to_string()))
    }

    pub fn supported(&self) -> Vec<Format> {
        let mut formats: Vec<_> = self.adapters.keys().copied().collect();
        formats.sort_by_key(|f| f.as_str());
        formats
    }
}

impl Default for FormatRegistry {
    fn default() -> Self {
        Self::with_defaults()
    }
}

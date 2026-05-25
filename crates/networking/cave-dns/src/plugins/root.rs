// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Root plugin — per-server root directory for relative zone paths.
//!
//! Upstream `plugin/root/root.go` resolves relative file paths used by other
//! plugins (file, hosts, secondary) against a server-scoped root directory.
//! cave-dns keeps the same semantics: plugins that take a path consult the
//! configured root via [`RootPlugin::resolve`] before opening the file.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::{
    error::{DnsError, DnsResult},
    plugins::{Next, Plugin, QueryContext},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RootConfig {
    pub directory: String,
}

impl Default for RootConfig {
    fn default() -> Self {
        Self {
            directory: ".".into(),
        }
    }
}

pub struct RootPlugin {
    config: RootConfig,
}

impl RootPlugin {
    pub fn new(config: RootConfig) -> DnsResult<Self> {
        if config.directory.is_empty() {
            return Err(DnsError::Config("root: directory must not be empty".into()));
        }
        Ok(Self { config })
    }

    /// Resolve `p` against the configured root directory. Absolute paths are
    /// returned unchanged; relative paths are joined with `directory`.
    pub fn resolve<P: AsRef<Path>>(&self, p: P) -> PathBuf {
        let p = p.as_ref();
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            PathBuf::from(&self.config.directory).join(p)
        }
    }

    pub fn directory(&self) -> &str {
        &self.config.directory
    }
}

#[async_trait]
impl Plugin for RootPlugin {
    fn name(&self) -> &str {
        "root"
    }

    async fn handle<'a>(&'a self, ctx: &mut QueryContext, next: Next<'a>) -> DnsResult<()> {
        // root is a setup-only directive; pass-through at request time.
        next.run(ctx).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_resolve_relative_joins_directory() {
        let p = RootPlugin::new(RootConfig {
            directory: "/etc/coredns".into(),
        })
        .unwrap();
        assert_eq!(p.resolve("zones/example.zone"), PathBuf::from("/etc/coredns/zones/example.zone"));
    }

    #[test]
    fn root_resolve_absolute_unchanged() {
        let p = RootPlugin::new(RootConfig {
            directory: "/etc/coredns".into(),
        })
        .unwrap();
        assert_eq!(p.resolve("/absolute/zone"), PathBuf::from("/absolute/zone"));
    }

    #[test]
    fn root_rejects_empty_directory() {
        let result = RootPlugin::new(RootConfig {
            directory: String::new(),
        });
        match result {
            Ok(_) => panic!("expected Err for empty directory"),
            Err(DnsError::Config(_)) => {}
            Err(other) => panic!("unexpected error variant: {other:?}"),
        }
    }

    #[test]
    fn root_default_is_cwd() {
        let p = RootPlugin::new(RootConfig::default()).unwrap();
        assert_eq!(p.directory(), ".");
        assert_eq!(p.name(), "root");
    }
}

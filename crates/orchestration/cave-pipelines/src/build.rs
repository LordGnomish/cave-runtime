// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Build strategies: Dockerfile, Buildpacks, Kaniko, S2I.
//!
//! Ports: pkg/image/build.go (Tekton Pipelines v0.55.0)

use serde::{Deserialize, Serialize};
use thiserror::Error;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BuildStrategy {
    Dockerfile,
    Buildpacks,
    Kaniko,
    S2i,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildArg {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheConfig {
    /// Images to use as cache sources.
    pub cache_from: Vec<String>,
    /// Whether to export a new cache image.
    pub cache_to: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildConfig {
    pub strategy: BuildStrategy,
    pub image: String,
    pub context: String,
    pub dockerfile: Option<String>,
    pub build_args: Vec<BuildArg>,
    pub cache: Option<CacheConfig>,
    pub push: bool,
    pub registry: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildResult {
    pub image_ref: String,
    pub digest: Option<String>,
    pub size_bytes: Option<u64>,
}

#[derive(Debug, Error)]
pub enum BuildError {
    #[error("Build failed: {0}")]
    Failed(String),
    #[error("Push failed for '{image}': {reason}")]
    PushFailed { image: String, reason: String },
    #[error("Strategy {0:?} is not supported in this environment")]
    UnsupportedStrategy(BuildStrategy),
}

/// A simple key/value parameter pair used for template interpolation.
#[derive(Debug, Clone)]
pub struct BuildParam {
    pub name: String,
    pub value: String,
}

// ---------------------------------------------------------------------------
// BuildConfig impl
// ---------------------------------------------------------------------------

impl BuildConfig {
    pub fn dockerfile(image: impl Into<String>, context: impl Into<String>) -> Self {
        Self {
            strategy: BuildStrategy::Dockerfile,
            image: image.into(),
            context: context.into(),
            dockerfile: Some("Dockerfile".to_string()),
            build_args: Vec::new(),
            cache: None,
            push: false,
            registry: None,
        }
    }

    pub fn kaniko(image: impl Into<String>, context: impl Into<String>) -> Self {
        Self {
            strategy: BuildStrategy::Kaniko,
            image: image.into(),
            context: context.into(),
            dockerfile: Some("Dockerfile".to_string()),
            build_args: Vec::new(),
            cache: None,
            push: true, // Kaniko always pushes
            registry: None,
        }
    }

    /// Return a copy with `$(params.NAME)` placeholders resolved.
    pub fn interpolated(&self, params: &[BuildParam]) -> Self {
        Self {
            image: interpolate(&self.image, params),
            context: interpolate(&self.context, params),
            dockerfile: self.dockerfile.as_deref().map(|d| interpolate(d, params)),
            build_args: self
                .build_args
                .iter()
                .map(|a| BuildArg {
                    name: a.name.clone(),
                    value: interpolate(&a.value, params),
                })
                .collect(),
            ..self.clone()
        }
    }

    /// Generate CLI arguments for the chosen build strategy.
    pub fn cli_args(&self) -> Vec<String> {
        match self.strategy {
            BuildStrategy::Dockerfile => self.docker_args(),
            BuildStrategy::Kaniko => self.kaniko_args(),
            BuildStrategy::Buildpacks => self.buildpacks_args(),
            BuildStrategy::S2i => self.s2i_args(),
        }
    }

    fn docker_args(&self) -> Vec<String> {
        let mut a = vec!["build".to_string(), "-t".to_string(), self.image.clone()];
        if let Some(df) = &self.dockerfile {
            a.extend(["-f".to_string(), df.clone()]);
        }
        for arg in &self.build_args {
            a.extend(["--build-arg".to_string(), format!("{}={}", arg.name, arg.value)]);
        }
        if let Some(cache) = &self.cache {
            for src in &cache.cache_from {
                a.extend(["--cache-from".to_string(), src.clone()]);
            }
        }
        a.push(self.context.clone());
        a
    }

    fn kaniko_args(&self) -> Vec<String> {
        let mut a = vec!["--destination".to_string(), self.image.clone()];
        if let Some(df) = &self.dockerfile {
            a.extend(["--dockerfile".to_string(), df.clone()]);
        }
        for arg in &self.build_args {
            a.extend(["--build-arg".to_string(), format!("{}={}", arg.name, arg.value)]);
        }
        if let Some(cache) = &self.cache {
            a.push("--cache=true".to_string());
            if let Some(to) = &cache.cache_to {
                a.extend(["--cache-repo".to_string(), to.clone()]);
            }
        }
        a.extend(["--context".to_string(), self.context.clone()]);
        a
    }

    fn buildpacks_args(&self) -> Vec<String> {
        vec![
            "build".to_string(),
            self.image.clone(),
            "--path".to_string(),
            self.context.clone(),
        ]
    }

    fn s2i_args(&self) -> Vec<String> {
        vec!["build".to_string(), self.context.clone(), self.image.clone()]
    }
}

/// Resolve `$(params.NAME)` placeholders in a template string.
fn interpolate(template: &str, params: &[BuildParam]) -> String {
    let mut result = template.to_string();
    for p in params {
        let placeholder = format!("$(params.{})", p.name);
        result = result.replace(&placeholder, &p.value);
    }
    result
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn param(name: &str, value: &str) -> BuildParam {
        BuildParam { name: name.to_string(), value: value.to_string() }
    }

    #[test]
    fn test_dockerfile_cli_args_contain_required_flags() {
        let cfg = BuildConfig::dockerfile("myapp:latest", ".");
        let args = cfg.cli_args();
        assert!(args.contains(&"build".to_string()));
        assert!(args.contains(&"-t".to_string()));
        assert!(args.contains(&"myapp:latest".to_string()));
        assert!(args.contains(&".".to_string()));
    }

    #[test]
    fn test_build_arg_included_in_docker_args() {
        let mut cfg = BuildConfig::dockerfile("img:v1", ".");
        cfg.build_args.push(BuildArg { name: "APP_VERSION".to_string(), value: "1.2".to_string() });
        let args = cfg.cli_args();
        assert!(args.contains(&"--build-arg".to_string()));
        assert!(args.contains(&"APP_VERSION=1.2".to_string()));
    }

    #[test]
    fn test_kaniko_args_contain_destination() {
        let cfg = BuildConfig::kaniko("registry/app:sha", "/workspace/source");
        let args = cfg.cli_args();
        assert!(args.contains(&"--destination".to_string()));
        assert!(args.contains(&"registry/app:sha".to_string()));
    }

    #[test]
    fn test_build_config_interpolation() {
        let cfg = BuildConfig::dockerfile("$(params.registry)/$(params.app):$(params.tag)", ".");
        let params = vec![
            param("registry", "ghcr.io/acme"),
            param("app", "backend"),
            param("tag", "v3.1"),
        ];
        let resolved = cfg.interpolated(&params);
        assert_eq!(resolved.image, "ghcr.io/acme/backend:v3.1");
    }

    #[test]
    fn test_buildpacks_args() {
        let cfg = BuildConfig {
            strategy: BuildStrategy::Buildpacks,
            image: "myapp:bp".to_string(),
            context: "./src".to_string(),
            dockerfile: None,
            build_args: vec![],
            cache: None,
            push: false,
            registry: None,
        };
        let args = cfg.cli_args();
        assert_eq!(args[0], "build");
        assert!(args.contains(&"myapp:bp".to_string()));
        assert!(args.contains(&"./src".to_string()));
    }
}

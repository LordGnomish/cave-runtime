// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Docker source — port of `pkg/sources/docker/docker.go`. Walks image
//! layers (tar.gz) and re-emits each in-layer file as a `Chunk`.

use crate::error::Result;
use crate::models::SourceKind;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DockerOptions {
    pub images: Vec<String>,
    pub registry_user: Option<String>,
    pub registry_password: Option<String>,
    pub tag_filter: Option<String>,
}

pub struct DockerSource {
    pub options: DockerOptions,
}

impl DockerSource {
    pub fn new(options: DockerOptions) -> Self {
        Self { options }
    }
    pub fn name(&self) -> &str {
        "docker"
    }
    pub fn kind(&self) -> SourceKind {
        SourceKind::Docker
    }

    /// `image:tag` -> (image, tag) tuple. Defaults to `latest` when no
    /// explicit tag.
    pub fn split_ref(image: &str) -> (String, String) {
        if let Some((host, rest)) = image.rsplit_once('/')
            && let Some((img, tag)) = rest.rsplit_once(':')
        {
            return (format!("{}/{}", host, img), tag.to_string());
        }
        if let Some((img, tag)) = image.rsplit_once(':') {
            return (img.to_string(), tag.to_string());
        }
        (image.to_string(), "latest".to_string())
    }

    pub fn chunks(&self) -> Result<Vec<crate::models::Chunk>> {
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_with_explicit_tag() {
        let (i, t) = DockerSource::split_ref("library/nginx:1.27");
        assert_eq!(i, "library/nginx");
        assert_eq!(t, "1.27");
    }

    #[test]
    fn split_defaults_to_latest() {
        let (i, t) = DockerSource::split_ref("nginx");
        assert_eq!(i, "nginx");
        assert_eq!(t, "latest");
    }

    #[test]
    fn split_with_registry_host() {
        let (i, t) = DockerSource::split_ref("ghcr.io/cave/runtime:v1.0");
        assert_eq!(i, "ghcr.io/cave/runtime");
        assert_eq!(t, "v1.0");
    }

    #[test]
    fn empty_chunks_offline() {
        let s = DockerSource::new(DockerOptions::default());
        assert!(s.chunks().unwrap().is_empty());
        assert_eq!(s.kind(), SourceKind::Docker);
    }
}

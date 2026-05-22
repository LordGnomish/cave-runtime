// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Service discovery backends: static, file-based, and Kubernetes.

use super::target::{FileSdConfig, KubernetesSdConfig, ScrapeConfig, ScrapeTarget, StaticConfig};
use crate::error::Result;
use crate::model::Labels;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Resolve static configs into targets.
pub fn resolve_static(
    job_name: &str,
    config: &ScrapeConfig,
    sc: &StaticConfig,
) -> Vec<ScrapeTarget> {
    sc.targets
        .iter()
        .map(|host| {
            let url = format!("{}://{}{}", config.scheme, host, config.metrics_path);

            let mut labels = sc.labels.clone();
            labels.insert("job", job_name);
            labels.insert("instance", host.as_str());

            ScrapeTarget::new(url, labels, config.clone())
        })
        .collect()
}

/// Resolve file-based SD (reads a JSON/YAML file with target groups).
pub fn resolve_file_sd(
    job_name: &str,
    config: &ScrapeConfig,
    fsc: &FileSdConfig,
) -> Vec<ScrapeTarget> {
    let mut targets = Vec::new();
    for file_pattern in &fsc.files {
        // Simple glob expansion: for now treat as literal paths.
        if let Ok(content) = std::fs::read_to_string(file_pattern) {
            if let Ok(groups) = serde_json::from_str::<Vec<FileSdGroup>>(&content) {
                for group in groups {
                    let sc = StaticConfig {
                        targets: group.targets,
                        labels: Labels::from_pairs(group.labels.into_iter()),
                    };
                    targets.extend(resolve_static(job_name, config, &sc));
                }
            }
        }
    }
    targets
}

#[derive(Debug, Deserialize)]
struct FileSdGroup {
    targets: Vec<String>,
    #[serde(default)]
    labels: std::collections::HashMap<String, String>,
}

/// Resolve all configured service discovery methods.
pub fn resolve_all(config: &ScrapeConfig) -> Vec<ScrapeTarget> {
    let mut targets = Vec::new();

    for sc in &config.static_configs {
        targets.extend(resolve_static(&config.job_name, config, sc));
    }

    for fsc in &config.file_sd_configs {
        targets.extend(resolve_file_sd(&config.job_name, config, fsc));
    }

    // Kubernetes SD: in a real system this would watch the K8s API.
    // Here we register the target structure but don't make live K8s calls.
    for ksc in &config.kubernetes_sd_configs {
        // Emit a placeholder to keep the structure.
        tracing::info!(
            "Kubernetes SD configured for role {:?} (namespaces: {:?})",
            ksc.role,
            ksc.namespaces
        );
    }

    targets
}

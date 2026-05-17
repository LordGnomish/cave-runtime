// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Provider store.

use crate::error::{CrossplaneError, CrossplaneResult};
use crate::models::{CreateProviderRequest, Provider, ProviderStatus, ProviderType};
use chrono::Utc;
use dashmap::DashMap;
use uuid::Uuid;

pub struct ProviderStore {
    providers: DashMap<String, Provider>,
    /// api_group → Vec of provider names
    managed_type_index: DashMap<String, Vec<String>>,
}

impl ProviderStore {
    pub fn new() -> Self {
        Self {
            providers: DashMap::new(),
            managed_type_index: DashMap::new(),
        }
    }

    /// Seed known managed resource types based on provider name patterns.
    fn seed_managed_types(name: &str) -> Vec<String> {
        if name.contains("aws") {
            vec![
                "s3.aws.crossplane.io/Bucket".into(),
                "rds.aws.crossplane.io/DBInstance".into(),
                "ec2.aws.crossplane.io/Instance".into(),
                "iam.aws.crossplane.io/Role".into(),
                "eks.aws.crossplane.io/Cluster".into(),
            ]
        } else if name.contains("gcp") {
            vec![
                "storage.gcp.crossplane.io/Bucket".into(),
                "sql.gcp.crossplane.io/DatabaseInstance".into(),
                "compute.gcp.crossplane.io/Instance".into(),
                "iam.gcp.crossplane.io/ServiceAccount".into(),
                "container.gcp.crossplane.io/Cluster".into(),
            ]
        } else if name.contains("azure") {
            vec![
                "storage.azure.crossplane.io/Account".into(),
                "database.azure.crossplane.io/PostgreSQLServer".into(),
                "compute.azure.crossplane.io/VirtualMachine".into(),
                "network.azure.crossplane.io/VirtualNetwork".into(),
                "containerservice.azure.crossplane.io/KubernetesCluster".into(),
            ]
        } else if name.contains("kubernetes") {
            vec![
                "kubernetes.crossplane.io/Object".into(),
                "kubernetes.crossplane.io/ProviderConfig".into(),
            ]
        } else if name.contains("helm") {
            vec![
                "helm.crossplane.io/Release".into(),
                "helm.crossplane.io/ProviderConfig".into(),
            ]
        } else {
            vec![]
        }
    }

    pub fn install(&self, req: CreateProviderRequest) -> CrossplaneResult<Provider> {
        if self.providers.contains_key(&req.name) {
            return Err(CrossplaneError::Internal(format!(
                "Provider already installed: {}",
                req.name
            )));
        }

        let managed_types = Self::seed_managed_types(&req.name);

        let provider = Provider {
            id: Uuid::new_v4(),
            name: req.name.clone(),
            package: req.package.clone(),
            provider_type: req.provider_type,
            revision: "v0.1.0".to_owned(),
            status: ProviderStatus::Installed,
            managed_resource_types: managed_types.clone(),
            created_at: Utc::now(),
        };

        // Index managed types by api_group (prefix before '/')
        for mrt in &managed_types {
            if let Some(group) = mrt.split('/').next() {
                self.managed_type_index
                    .entry(group.to_owned())
                    .or_default()
                    .push(req.name.clone());
            }
        }

        self.providers.insert(req.name, provider.clone());
        Ok(provider)
    }

    pub fn get(&self, name: &str) -> CrossplaneResult<Provider> {
        self.providers
            .get(name)
            .map(|r| r.clone())
            .ok_or_else(|| CrossplaneError::ProviderNotFound(name.to_owned()))
    }

    pub fn list(&self) -> Vec<Provider> {
        self.providers.iter().map(|r| r.value().clone()).collect()
    }

    pub fn delete(&self, name: &str) -> CrossplaneResult<()> {
        match self.providers.remove(name) {
            Some((_, provider)) => {
                for mrt in &provider.managed_resource_types {
                    if let Some(group) = mrt.split('/').next() {
                        if let Some(mut names) = self.managed_type_index.get_mut(group) {
                            names.retain(|n| n != name);
                        }
                    }
                }
                Ok(())
            }
            None => Err(CrossplaneError::ProviderNotFound(name.to_owned())),
        }
    }

    pub fn mark_healthy(&self, name: &str) -> CrossplaneResult<()> {
        match self.providers.get_mut(name) {
            Some(mut p) => {
                p.status = ProviderStatus::Installed;
                Ok(())
            }
            None => Err(CrossplaneError::ProviderNotFound(name.to_owned())),
        }
    }

    /// Return well-known providers as a catalog.
    pub fn catalog(&self) -> Vec<Provider> {
        let catalog_entries = vec![
            ("provider-aws", "xpkg.upbound.io/upbound/provider-aws:latest", ProviderType::Official),
            ("provider-gcp", "xpkg.upbound.io/upbound/provider-gcp:latest", ProviderType::Official),
            ("provider-azure", "xpkg.upbound.io/upbound/provider-azure:latest", ProviderType::Official),
            ("provider-kubernetes", "xpkg.upbound.io/crossplane-contrib/provider-kubernetes:latest", ProviderType::Community),
            ("provider-helm", "xpkg.upbound.io/crossplane-contrib/provider-helm:latest", ProviderType::Community),
        ];

        catalog_entries
            .into_iter()
            .map(|(name, package, provider_type)| {
                let managed_types = Self::seed_managed_types(name);
                Provider {
                    id: Uuid::new_v4(),
                    name: name.to_owned(),
                    package: package.to_owned(),
                    provider_type,
                    revision: "latest".to_owned(),
                    status: ProviderStatus::NotInstalled,
                    managed_resource_types: managed_types,
                    created_at: Utc::now(),
                }
            })
            .collect()
    }
}

impl Default for ProviderStore {
    fn default() -> Self {
        Self::new()
    }
}

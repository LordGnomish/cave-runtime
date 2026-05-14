// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Runtime configuration — loaded from YAML, env vars, and Kubernetes ConfigMaps.
//!
//! Configuration resolves in this order:
//! 1. Profile defaults (from DeploymentProfile)
//! 2. YAML config file overrides
//! 3. Environment variable overrides (CAVE_ prefix)
//! 4. CLI argument overrides

use crate::profile::{DeploymentProfile, Environment, Provider};
use serde::Deserialize;

/// Top-level configuration for the CAVE Unified Runtime.
#[derive(Debug, Deserialize, Clone)]
pub struct CaveConfig {
    /// Active deployment profile
    #[serde(default)]
    pub profile: ProfileSelector,
    /// Runtime server settings
    pub server: ServerConfig,
    /// Authentication provider settings
    pub auth: AuthConfig,
    /// Database connection settings
    pub database: DatabaseConfig,
    /// Module enable/disable flags
    pub modules: ModuleConfig,
    /// Storage backend configuration
    #[serde(default)]
    pub storage: StorageConfig,
}

/// Profile selector in config YAML.
#[derive(Debug, Deserialize, Clone)]
#[serde(untagged)]
pub enum ProfileSelector {
    /// Simple name: "hetzner-dev", "azure-prod", "local"
    Name(String),
    /// Explicit fields
    Explicit {
        environment: Option<Environment>,
        provider: Option<Provider>,
    },
}

impl Default for ProfileSelector {
    fn default() -> Self {
        Self::Name("local".to_string())
    }
}

impl ProfileSelector {
    /// Resolve to a DeploymentProfile.
    pub fn resolve(&self) -> Result<DeploymentProfile, crate::CaveError> {
        match self {
            Self::Name(name) => match name.as_str() {
                "local" => Ok(DeploymentProfile::local()),
                "hetzner-dev" => Ok(DeploymentProfile::new(Environment::Dev, Provider::Hetzner)),
                "hetzner-staging" => {
                    Ok(DeploymentProfile::new(Environment::Staging, Provider::Hetzner))
                }
                "hetzner-prod" => {
                    Ok(DeploymentProfile::new(Environment::Prod, Provider::Hetzner))
                }
                "azure-dev" => Ok(DeploymentProfile::new(Environment::Dev, Provider::Azure)),
                "azure-staging" => {
                    Ok(DeploymentProfile::new(Environment::Staging, Provider::Azure))
                }
                "azure-prod" => Ok(DeploymentProfile::new(Environment::Prod, Provider::Azure)),
                other => Err(crate::CaveError::Config(format!(
                    "Unknown profile: {other}. Valid profiles: local, hetzner-dev, hetzner-staging, hetzner-prod, azure-dev, azure-staging, azure-prod"
                ))),
            },
            Self::Explicit {
                environment,
                provider,
            } => match (environment, provider) {
                (Some(env), Some(prov)) => Ok(DeploymentProfile::new(*env, *prov)),
                (None, None) => Ok(DeploymentProfile::local()),
                _ => Err(crate::CaveError::Config(
                    "Profile requires both environment and provider, or neither (for local)"
                        .to_string(),
                )),
            },
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct ServerConfig {
    /// Listen address (default: 0.0.0.0)
    pub host: String,
    /// Listen port (default: 8080)
    pub port: u16,
    /// Metrics port (default: 9090)
    pub metrics_port: u16,
}

#[derive(Debug, Deserialize, Clone)]
pub struct AuthConfig {
    /// OIDC provider: "okta" or "keycloak"
    pub provider: AuthProvider,
    /// OIDC issuer URL (e.g., https://keycloak.cave.caveplatform.dev/realms/cave)
    pub issuer_url: String,
    /// OIDC audience
    pub audience: String,
    /// JWKS URI (auto-discovered from issuer if not set)
    pub jwks_uri: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "lowercase")]
pub enum AuthProvider {
    Okta,
    Keycloak,
}

#[derive(Debug, Deserialize, Clone)]
pub struct DatabaseConfig {
    /// PostgreSQL connection URL
    pub url: String,
    /// Max pool size (default: 20)
    pub max_pool_size: Option<u32>,
    /// Enable Row-Level Security enforcement
    #[serde(default)]
    pub enable_rls: bool,
}

#[derive(Debug, Deserialize, Clone)]
pub struct StorageConfig {
    /// Storage backend type
    #[serde(default)]
    pub backend: StorageBackend,
    /// S3/MinIO endpoint (for object storage)
    pub s3_endpoint: Option<String>,
    /// S3 bucket name
    pub s3_bucket: Option<String>,
    /// Azure storage account (for ADLS Gen2)
    pub azure_account: Option<String>,
    /// Azure container
    pub azure_container: Option<String>,
}

#[derive(Debug, Deserialize, Clone, Default)]
#[serde(rename_all = "lowercase")]
pub enum StorageBackend {
    /// MinIO (Hetzner/local)
    #[default]
    Minio,
    /// Azure Data Lake Storage Gen2
    Adls,
    /// Local filesystem (development only)
    Local,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            backend: StorageBackend::default(),
            s3_endpoint: None,
            s3_bucket: None,
            azure_account: None,
            azure_container: None,
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct ModuleConfig {
    pub flags: bool,
    pub secrets: bool,
    pub lint: bool,
    pub docs: bool,
    pub status: bool,
    pub changelog: bool,
    pub vulns: bool,
    pub sbom: bool,
    pub scan: bool,
    pub registry: bool,
    pub pii: bool,
    pub portal: bool,
    pub devlake: bool,
    pub uptime: bool,
    pub cost: bool,
    pub ai_obs: bool,
    pub workflows: bool,
    pub chat: bool,
    pub incidents: bool,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "0.0.0.0".into(),
            port: 8080,
            metrics_port: 9090,
        }
    }
}

impl Default for ModuleConfig {
    fn default() -> Self {
        Self {
            flags: true,
            secrets: true,
            lint: true,
            docs: true,
            status: true,
            changelog: true,
            vulns: false,
            sbom: false,
            scan: false,
            registry: false,
            pii: false,
            portal: false,
            devlake: false,
            uptime: false,
            cost: false,
            ai_obs: false,
            workflows: false,
            chat: false,
            incidents: false,
        }
    }
}

impl CaveConfig {
    /// Load config from file, with env var overrides.
    pub fn load(path: &std::path::Path) -> Result<Self, crate::CaveError> {
        let contents = std::fs::read_to_string(path).map_err(|e| {
            crate::CaveError::Config(format!("Failed to read config {}: {e}", path.display()))
        })?;
        let mut config: Self = serde_yaml::from_str(&contents).map_err(|e| {
            crate::CaveError::Config(format!("Failed to parse config: {e}"))
        })?;

        // Environment variable overrides (CAVE_ prefix)
        if let Ok(port) = std::env::var("CAVE_PORT") {
            if let Ok(p) = port.parse() {
                config.server.port = p;
            }
        }
        if let Ok(db_url) = std::env::var("CAVE_DATABASE_URL") {
            config.database.url = db_url;
        }
        if let Ok(profile) = std::env::var("CAVE_PROFILE") {
            config.profile = ProfileSelector::Name(profile);
        }

        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_profile_resolve_local() {
        let sel = ProfileSelector::Name("local".to_string());
        let profile = sel.resolve().unwrap();
        assert!(profile.is_local());
    }

    #[test]
    fn test_profile_resolve_hetzner_prod() {
        let sel = ProfileSelector::Name("hetzner-prod".to_string());
        let profile = sel.resolve().unwrap();
        assert!(profile.is_production());
        assert_eq!(profile.provider(), Some(Provider::Hetzner));
    }

    #[test]
    fn test_profile_resolve_invalid() {
        let sel = ProfileSelector::Name("invalid".to_string());
        assert!(sel.resolve().is_err());
    }

    #[test]
    fn test_default_profile_is_local() {
        let sel = ProfileSelector::default();
        let profile = sel.resolve().unwrap();
        assert!(profile.is_local());
    }
}

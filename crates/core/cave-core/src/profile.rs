// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Deployment profile system — 7 profiles (3 envs × 2 providers + local).
//!
//! Implements Principle 3 (Profile-Driven Deployment) and ADR-094.
//! Same cave-ctl commands produce different infrastructure based on active profile.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;

/// Cloud provider target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Hetzner,
    Azure,
}

/// Environment tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Environment {
    Dev,
    Staging,
    Prod,
}

/// One of the 7 deployment profiles.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum DeploymentProfile {
    /// Cloud profile: env × provider
    Cloud {
        environment: Environment,
        provider: Provider,
    },
    /// Local development profile (vcluster on Docker/kind)
    Local,
}

impl DeploymentProfile {
    pub fn new(environment: Environment, provider: Provider) -> Self {
        Self::Cloud {
            environment,
            provider,
        }
    }

    pub fn local() -> Self {
        Self::Local
    }

    /// Canonical name: "hetzner-dev", "azure-prod", "local"
    pub fn name(&self) -> String {
        match self {
            Self::Cloud {
                environment,
                provider,
            } => format!("{provider}-{environment}"),
            Self::Local => "local".to_string(),
        }
    }

    pub fn environment(&self) -> Environment {
        match self {
            Self::Cloud { environment, .. } => *environment,
            Self::Local => Environment::Dev,
        }
    }

    pub fn provider(&self) -> Option<Provider> {
        match self {
            Self::Cloud { provider, .. } => Some(*provider),
            Self::Local => None,
        }
    }

    pub fn is_production(&self) -> bool {
        self.environment() == Environment::Prod
    }

    pub fn is_local(&self) -> bool {
        matches!(self, Self::Local)
    }

    /// Whether this profile uses managed services (Azure) vs self-hosted (Hetzner/local).
    pub fn uses_managed_services(&self) -> bool {
        self.provider() == Some(Provider::Azure)
    }
}

impl fmt::Display for Provider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Hetzner => write!(f, "hetzner"),
            Self::Azure => write!(f, "azure"),
        }
    }
}

impl fmt::Display for Environment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Dev => write!(f, "dev"),
            Self::Staging => write!(f, "staging"),
            Self::Prod => write!(f, "prod"),
        }
    }
}

impl fmt::Display for DeploymentProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}

/// Resource sizing per profile. Implements ADR-094 topology definitions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileResources {
    /// Control plane node count (0 for AKS managed, 1 for local, 3 for prod)
    pub control_plane_nodes: u32,
    /// Worker node count
    pub worker_nodes: u32,
    /// CPU per worker node
    pub cpu_per_node: u32,
    /// RAM per worker node (GiB)
    pub ram_per_node_gib: u32,
    /// PostgreSQL max connections
    pub pg_max_connections: u32,
    /// PostgreSQL pool size
    pub pg_pool_size: u32,
    /// Cache memory limit (MiB)
    pub cache_memory_mib: u32,
}

impl ProfileResources {
    /// Default resource sizing per profile (from Runbook §04).
    pub fn for_profile(profile: &DeploymentProfile) -> Self {
        match profile {
            DeploymentProfile::Local => Self {
                control_plane_nodes: 1,
                worker_nodes: 1,
                cpu_per_node: 2,
                ram_per_node_gib: 4,
                pg_max_connections: 100,
                pg_pool_size: 5,
                cache_memory_mib: 256,
            },
            DeploymentProfile::Cloud {
                environment: Environment::Dev,
                provider: Provider::Hetzner,
            } => Self {
                control_plane_nodes: 1,
                worker_nodes: 2,
                cpu_per_node: 2,
                ram_per_node_gib: 4,
                pg_max_connections: 200,
                pg_pool_size: 10,
                cache_memory_mib: 512,
            },
            DeploymentProfile::Cloud {
                environment: Environment::Dev,
                provider: Provider::Azure,
            } => Self {
                control_plane_nodes: 0, // AKS managed
                worker_nodes: 2,
                cpu_per_node: 2,
                ram_per_node_gib: 4,
                pg_max_connections: 200,
                pg_pool_size: 10,
                cache_memory_mib: 512,
            },
            DeploymentProfile::Cloud {
                environment: Environment::Staging,
                provider: Provider::Hetzner,
            } => Self {
                control_plane_nodes: 3,
                worker_nodes: 5,
                cpu_per_node: 4,
                ram_per_node_gib: 8,
                pg_max_connections: 500,
                pg_pool_size: 20,
                cache_memory_mib: 1024,
            },
            DeploymentProfile::Cloud {
                environment: Environment::Staging,
                provider: Provider::Azure,
            } => Self {
                control_plane_nodes: 0,
                worker_nodes: 5,
                cpu_per_node: 4,
                ram_per_node_gib: 8,
                pg_max_connections: 500,
                pg_pool_size: 20,
                cache_memory_mib: 1024,
            },
            DeploymentProfile::Cloud {
                environment: Environment::Prod,
                provider: Provider::Hetzner,
            } => Self {
                control_plane_nodes: 3,
                worker_nodes: 10,
                cpu_per_node: 8,
                ram_per_node_gib: 16,
                pg_max_connections: 2000,
                pg_pool_size: 50,
                cache_memory_mib: 4096,
            },
            DeploymentProfile::Cloud {
                environment: Environment::Prod,
                provider: Provider::Azure,
            } => Self {
                control_plane_nodes: 0,
                worker_nodes: 10,
                cpu_per_node: 8,
                ram_per_node_gib: 16,
                pg_max_connections: 5000,
                pg_pool_size: 50,
                cache_memory_mib: 4096,
            },
        }
    }
}

/// Which modules are enabled per profile. Dev/local = minimal, prod = full.
/// Implements Principle 3 and the phased rollout from One-Prompt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileModules {
    // Phase 1: Core (always enabled)
    pub gateway: bool,
    pub mesh: bool,
    pub identity: bool,
    pub secrets: bool,
    pub observability: bool,
    pub gitops: bool,
    pub portal: bool,

    // Phase 2: Data/AI
    pub postgresql: bool,
    pub kafka: bool,
    pub object_storage: bool,
    pub cache: bool,
    pub search: bool,
    pub vector_search: bool,
    pub llm: bool,
    pub mlops: bool,

    // Phase 3: Advanced
    pub security_pipeline: bool,
    pub chaos: bool,
    pub workflows: bool,
    pub dora: bool,
    pub finops: bool,
    pub backup: bool,
    pub forensics: bool,
    pub apol: bool,
    pub pam: bool,

    // Phase 4: Extensions
    pub serverless: bool,
}

impl ProfileModules {
    /// Default module set per profile.
    pub fn for_profile(profile: &DeploymentProfile) -> Self {
        match profile.environment() {
            Environment::Dev => Self {
                // Phase 1: Core — always on
                gateway: true,
                mesh: true,
                identity: true,
                secrets: true,
                observability: true,
                gitops: true,
                portal: true,
                // Phase 2: Data — minimal
                postgresql: true,
                kafka: false,
                object_storage: true,
                cache: true,
                search: false,
                vector_search: false,
                llm: false,
                mlops: false,
                // Phase 3: Off
                security_pipeline: false,
                chaos: false,
                workflows: false,
                dora: false,
                finops: false,
                backup: false,
                forensics: false,
                apol: false,
                pam: false,
                // Phase 4: Off
                serverless: false,
            },
            Environment::Staging => Self {
                gateway: true,
                mesh: true,
                identity: true,
                secrets: true,
                observability: true,
                gitops: true,
                portal: true,
                postgresql: true,
                kafka: true,
                object_storage: true,
                cache: true,
                search: true,
                vector_search: false,
                llm: true,
                mlops: false,
                security_pipeline: true,
                chaos: true,
                workflows: true,
                dora: true,
                finops: true,
                backup: true,
                forensics: true,
                apol: false,
                pam: true,
                serverless: false,
            },
            Environment::Prod => Self {
                gateway: true,
                mesh: true,
                identity: true,
                secrets: true,
                observability: true,
                gitops: true,
                portal: true,
                postgresql: true,
                kafka: true,
                object_storage: true,
                cache: true,
                search: true,
                vector_search: true,
                llm: true,
                mlops: true,
                security_pipeline: true,
                chaos: true,
                workflows: true,
                dora: true,
                finops: true,
                backup: true,
                forensics: true,
                apol: true,
                pam: true,
                serverless: false, // Phase 4 opt-in only
            },
        }
    }

    /// Local profile: absolute minimum for developer iteration.
    pub fn local() -> Self {
        Self {
            gateway: true,
            mesh: false, // ztunnel only via cave-ctl mesh full
            identity: true,
            secrets: true,
            observability: true,
            gitops: false,
            portal: true,
            postgresql: true,
            kafka: false,
            object_storage: true,
            cache: true,
            search: false,
            vector_search: false,
            llm: false,
            mlops: false,
            security_pipeline: false,
            chaos: false,
            workflows: false,
            dora: false,
            finops: false,
            backup: false,
            forensics: false,
            apol: false,
            pam: false,
            serverless: false,
        }
    }
}

/// Identity provider configuration, derived from profile.
/// Implements ADR-064 (Identity Split).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileIdentity {
    /// User OIDC provider
    pub oidc_provider: OidcProvider,
    /// PAM provider
    pub pam_provider: PamProvider,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OidcProvider {
    Keycloak,
    Okta,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PamProvider {
    Teleport,
    CyberArk,
}

impl ProfileIdentity {
    pub fn for_profile(profile: &DeploymentProfile) -> Self {
        match profile.provider() {
            Some(Provider::Azure) => Self {
                oidc_provider: OidcProvider::Okta,
                pam_provider: PamProvider::CyberArk,
            },
            Some(Provider::Hetzner) | None => Self {
                oidc_provider: OidcProvider::Keycloak,
                pam_provider: PamProvider::Teleport,
            },
        }
    }
}

/// Database backend selection, derived from profile.
/// Implements ADR-047.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DatabaseBackend {
    /// Self-hosted via CloudNativePG operator (Hetzner/local)
    CloudNativePg,
    /// Azure Database for PostgreSQL Flexible Server
    AzurePgFlexible,
    /// Embedded SQLite for local/edge (from ARCHITECTURE-ELASTIC-SCALE.md Tier 1)
    Sqlite,
}

impl DatabaseBackend {
    pub fn for_profile(profile: &DeploymentProfile) -> Self {
        match profile {
            DeploymentProfile::Local => Self::Sqlite,
            DeploymentProfile::Cloud {
                provider: Provider::Azure,
                ..
            } => Self::AzurePgFlexible,
            DeploymentProfile::Cloud {
                provider: Provider::Hetzner,
                ..
            } => Self::CloudNativePg,
        }
    }
}

/// Full profile configuration — resolved from profile + overrides.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileConfig {
    pub profile: DeploymentProfile,
    pub resources: ProfileResources,
    pub modules: ProfileModules,
    pub identity: ProfileIdentity,
    pub database_backend: DatabaseBackend,
    /// Domain (default: caveplatform.dev)
    pub domain: String,
    /// Platform wildcard: *.cave.<domain>
    pub platform_wildcard: String,
    /// API wildcard: *.api.<domain>
    pub api_wildcard: String,
}

impl ProfileConfig {
    /// Build a complete profile config from a deployment profile with defaults.
    pub fn from_profile(profile: DeploymentProfile) -> Self {
        let resources = ProfileResources::for_profile(&profile);
        let modules = match &profile {
            DeploymentProfile::Local => ProfileModules::local(),
            _ => ProfileModules::for_profile(&profile),
        };
        let identity = ProfileIdentity::for_profile(&profile);
        let database_backend = DatabaseBackend::for_profile(&profile);
        let domain = "caveplatform.dev".to_string();

        Self {
            platform_wildcard: format!("*.cave.{domain}"),
            api_wildcard: format!("*.api.{domain}"),
            profile,
            resources,
            modules,
            identity,
            database_backend,
            domain,
        }
    }

    /// Load from YAML file, falling back to defaults for missing fields.
    pub fn load(path: &std::path::Path) -> Result<Self, crate::CaveError> {
        let contents = std::fs::read_to_string(path).map_err(|e| {
            crate::CaveError::Config(format!(
                "Failed to read profile config {}: {e}",
                path.display()
            ))
        })?;
        serde_yaml::from_str(&contents)
            .map_err(|e| crate::CaveError::Config(format!("Failed to parse profile config: {e}")))
    }

    /// Profile config file path: ~/.cave/profiles/<name>.yaml
    pub fn config_path(&self) -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
        PathBuf::from(home)
            .join(".cave")
            .join("profiles")
            .join(format!("{}.yaml", self.profile.name()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_profile_names() {
        let p = DeploymentProfile::new(Environment::Dev, Provider::Hetzner);
        assert_eq!(p.name(), "hetzner-dev");

        let p = DeploymentProfile::new(Environment::Prod, Provider::Azure);
        assert_eq!(p.name(), "azure-prod");

        let p = DeploymentProfile::local();
        assert_eq!(p.name(), "local");
    }

    #[test]
    fn test_seven_profiles_exist() {
        let profiles = vec![
            DeploymentProfile::new(Environment::Dev, Provider::Hetzner),
            DeploymentProfile::new(Environment::Dev, Provider::Azure),
            DeploymentProfile::new(Environment::Staging, Provider::Hetzner),
            DeploymentProfile::new(Environment::Staging, Provider::Azure),
            DeploymentProfile::new(Environment::Prod, Provider::Hetzner),
            DeploymentProfile::new(Environment::Prod, Provider::Azure),
            DeploymentProfile::local(),
        ];
        assert_eq!(profiles.len(), 7);
        // All names unique
        let names: std::collections::HashSet<_> = profiles.iter().map(|p| p.name()).collect();
        assert_eq!(names.len(), 7);
    }

    #[test]
    fn test_hetzner_uses_keycloak() {
        let p = DeploymentProfile::new(Environment::Prod, Provider::Hetzner);
        let id = ProfileIdentity::for_profile(&p);
        assert!(matches!(id.oidc_provider, OidcProvider::Keycloak));
        assert!(matches!(id.pam_provider, PamProvider::Teleport));
    }

    #[test]
    fn test_azure_uses_okta() {
        let p = DeploymentProfile::new(Environment::Prod, Provider::Azure);
        let id = ProfileIdentity::for_profile(&p);
        assert!(matches!(id.oidc_provider, OidcProvider::Okta));
        assert!(matches!(id.pam_provider, PamProvider::CyberArk));
    }

    #[test]
    fn test_local_uses_sqlite() {
        let p = DeploymentProfile::local();
        let db = DatabaseBackend::for_profile(&p);
        assert!(matches!(db, DatabaseBackend::Sqlite));
    }

    #[test]
    fn test_hetzner_uses_cnpg() {
        let p = DeploymentProfile::new(Environment::Prod, Provider::Hetzner);
        let db = DatabaseBackend::for_profile(&p);
        assert!(matches!(db, DatabaseBackend::CloudNativePg));
    }

    #[test]
    fn test_dev_has_minimal_modules() {
        let p = DeploymentProfile::new(Environment::Dev, Provider::Hetzner);
        let m = ProfileModules::for_profile(&p);
        assert!(m.gateway);
        assert!(m.identity);
        assert!(!m.kafka);
        assert!(!m.chaos);
        assert!(!m.apol);
    }

    #[test]
    fn test_prod_has_all_modules_except_serverless() {
        let p = DeploymentProfile::new(Environment::Prod, Provider::Azure);
        let m = ProfileModules::for_profile(&p);
        assert!(m.gateway);
        assert!(m.kafka);
        assert!(m.apol);
        assert!(!m.serverless);
    }

    #[test]
    fn test_prod_hetzner_resources() {
        let p = DeploymentProfile::new(Environment::Prod, Provider::Hetzner);
        let r = ProfileResources::for_profile(&p);
        assert_eq!(r.control_plane_nodes, 3);
        assert!(r.worker_nodes >= 10);
        assert_eq!(r.cpu_per_node, 8);
    }

    #[test]
    fn test_azure_has_zero_control_plane() {
        let p = DeploymentProfile::new(Environment::Prod, Provider::Azure);
        let r = ProfileResources::for_profile(&p);
        assert_eq!(r.control_plane_nodes, 0); // AKS managed
    }

    #[test]
    fn test_profile_config_domain() {
        let p = ProfileConfig::from_profile(DeploymentProfile::new(
            Environment::Prod,
            Provider::Hetzner,
        ));
        assert_eq!(p.domain, "caveplatform.dev");
        assert_eq!(p.platform_wildcard, "*.cave.caveplatform.dev");
        assert_eq!(p.api_wildcard, "*.api.caveplatform.dev");
    }
}

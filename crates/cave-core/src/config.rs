//! Runtime configuration — loaded from YAML, env vars, and Kubernetes ConfigMaps.

use serde::Deserialize;

/// Top-level configuration for the CAVE Unified Runtime.
#[derive(Debug, Deserialize, Clone)]
pub struct CaveConfig {
    /// Runtime server settings
    pub server: ServerConfig,
    /// Authentication provider settings
    pub auth: AuthConfig,
    /// Database connection settings
    pub database: DatabaseConfig,
    /// Module enable/disable flags
    pub modules: ModuleConfig,
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
    /// OIDC issuer URL (e.g., https://knauf.okta.com or https://keycloak.cave.caveplatform.dev/realms/cave)
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

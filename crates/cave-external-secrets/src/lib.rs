//! CAVE External Secrets Operator — sync secrets from external providers.

pub mod error;
pub mod models;
pub mod provider;
pub mod routes;
pub mod store;
pub mod sync;

use axum::Router;
use std::sync::Arc;

pub use error::{EsoError, EsoResult};

pub const MODULE_NAME: &str = "external-secrets";

pub struct ExternalSecretsState {
    pub secret_stores: Arc<store::SecretStoreRegistry>,
    pub external_secrets: Arc<sync::ExternalSecretStore>,
    pub push_secrets: Arc<sync::PushSecretStore>,
    pub provider_configs: Arc<provider::ProviderConfigStore>,
}

impl Default for ExternalSecretsState {
    fn default() -> Self {
        Self {
            secret_stores: Arc::new(store::SecretStoreRegistry::new()),
            external_secrets: Arc::new(sync::ExternalSecretStore::new()),
            push_secrets: Arc::new(sync::PushSecretStore::new()),
            provider_configs: Arc::new(provider::ProviderConfigStore::new()),
        }
    }
}

pub fn router(state: Arc<ExternalSecretsState>) -> Router {
    routes::create_router(state)
}

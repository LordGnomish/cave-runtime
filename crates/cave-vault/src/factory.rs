//! Factory function for selecting the secrets engine from config.

use std::sync::Arc;

use crate::backend::{BuiltinSecretsEngine, SecretsEngine, SecretsEngineProfile};
use crate::adapters::{
    hashicorp::HashiCorpVaultAdapter,
    aws_sm::AwsSecretsManagerAdapter,
    azure_kv::AzureKeyVaultAdapter,
};
use crate::VaultState;

/// Instantiate the correct secrets engine for the given profile.
pub fn create_secrets_engine(
    profile: SecretsEngineProfile,
    state: Arc<VaultState>,
) -> Arc<dyn SecretsEngine> {
    match profile {
        SecretsEngineProfile::Builtin => {
            tracing::info!(backend = "builtin-vault", "secrets engine selected");
            Arc::new(BuiltinSecretsEngine::new(state))
        }

        SecretsEngineProfile::HashiCorpVault => {
            let config = crate::adapters::hashicorp::HashiCorpVaultConfig {
                addr: std::env::var("HCVAULT_ADDR")
                    .unwrap_or_else(|_| "https://127.0.0.1:8200".to_string()),
                token: std::env::var("HCVAULT_TOKEN").unwrap_or_default(),
                mount: std::env::var("HCVAULT_MOUNT")
                    .unwrap_or_else(|_| "secret".to_string()),
            };
            tracing::info!(backend = "hashicorp-vault", addr = %config.addr, "secrets engine selected");
            Arc::new(HashiCorpVaultAdapter::new(config))
        }

        SecretsEngineProfile::AwsSecretsManager => {
            let config = crate::adapters::aws_sm::AwsSecretsManagerConfig {
                region: std::env::var("AWS_REGION")
                    .or_else(|_| std::env::var("AWS_DEFAULT_REGION"))
                    .unwrap_or_else(|_| "us-east-1".to_string()),
                prefix: std::env::var("AWS_SM_PREFIX")
                    .unwrap_or_else(|_| "/cave/".to_string()),
            };
            tracing::info!(backend = "aws-secrets-manager", region = %config.region, "secrets engine selected");
            Arc::new(AwsSecretsManagerAdapter::new(config))
        }

        SecretsEngineProfile::AzureKeyVault => {
            let config = crate::adapters::azure_kv::AzureKeyVaultConfig {
                vault_url: std::env::var("AKV_VAULT_URL").unwrap_or_default(),
                tenant_id: std::env::var("AZURE_TENANT_ID").unwrap_or_default(),
                client_id: std::env::var("AZURE_CLIENT_ID").unwrap_or_default(),
                client_secret: std::env::var("AZURE_CLIENT_SECRET").unwrap_or_default(),
            };
            tracing::info!(backend = "azure-key-vault", vault_url = %config.vault_url, "secrets engine selected");
            Arc::new(AzureKeyVaultAdapter::new(config))
        }
    }
}

/// Convenience: build secrets engine from environment variables alone.
///
/// `CAVE_SECRETS_BACKEND` = `builtin` | `hashicorp_vault` | `aws_secrets_manager` | `azure_key_vault`
pub fn create_secrets_engine_from_env(state: Arc<VaultState>) -> Arc<dyn SecretsEngine> {
    let profile = match std::env::var("CAVE_SECRETS_BACKEND")
        .unwrap_or_else(|_| "builtin".to_string())
        .as_str()
    {
        "hashicorp_vault" | "hashicorp" | "vault" => SecretsEngineProfile::HashiCorpVault,
        "aws_secrets_manager" | "aws_sm" | "aws" => SecretsEngineProfile::AwsSecretsManager,
        "azure_key_vault" | "azure_kv" | "azure" => SecretsEngineProfile::AzureKeyVault,
        _ => SecretsEngineProfile::Builtin,
    };
    create_secrets_engine(profile, state)
}

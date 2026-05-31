// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Device provisioning — ThingsBoard `DeviceProvisionService` +
//! `DeviceProfileProvisionConfiguration`.
//!
//! A device profile carries a provisioning *strategy* keyed by a
//! `provision_device_key` / `provision_device_secret` pair. Unclaimed
//! devices present that pair (over MQTT/HTTP/CoAP) and the gateway either
//! creates a brand-new device (`AllowCreateNewDevices`) or validates that a
//! matching device was pre-provisioned (`CheckPreProvisionedDevices`), then
//! returns freshly-minted access-token credentials.

use crate::registry::{CredentialsType, DeviceCredentials, DeviceRegistry, TransportType};
use crate::{IotError, Result};
use std::collections::HashMap;

/// Provisioning strategy on a device profile (`DeviceProvisionType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProvisionStrategy {
    /// Provisioning rejected outright.
    Disabled,
    /// Any device presenting the key/secret gets created + credentialed.
    AllowCreateNewDevices,
    /// Only devices already created (pre-provisioned) may obtain credentials.
    CheckPreProvisionedDevices,
}

/// A provisioning credential bound to a tenant + device profile.
#[derive(Debug, Clone)]
pub struct ProvisionConfig {
    pub tenant_id: String,
    pub device_profile_id: String,
    pub strategy: ProvisionStrategy,
    pub provision_device_key: String,
    pub provision_device_secret: String,
}

/// Inbound provisioning request from a device.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProvisionRequest {
    pub device_name: String,
    pub provision_device_key: String,
    pub provision_device_secret: String,
}

/// Provisioning outcome (`ProvisionResponseStatus`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE", tag = "status")]
pub enum ProvisionResponse {
    /// Device provisioned — carries the issued access token.
    Success { device_id: String, access_token: String },
    /// Key/secret matched no configuration, or pre-provisioned check failed.
    NotFound,
    /// Strategy is `Disabled`.
    Failure { reason: String },
}

/// Provisioning service over a registry. Holds the per-(key) configs and a
/// monotonic token minter for deterministic tests.
#[derive(Debug, Default)]
pub struct ProvisionService {
    configs: HashMap<String, ProvisionConfig>,
    token_seq: u64,
}

/// Result of a bulk-provisioning batch.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BulkReport {
    pub created: usize,
    pub failed: usize,
}

impl ProvisionService {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a provisioning configuration, keyed by its device key.
    pub fn register_config(&mut self, config: ProvisionConfig) {
        self.configs
            .insert(config.provision_device_key.clone(), config);
    }

    fn next_token(&mut self) -> String {
        self.token_seq += 1;
        // Deterministic per service for testability; real deployments swap in
        // a CSPRNG-backed minter. Includes a uuid suffix for global uniqueness.
        format!("prov-{}-{}", self.token_seq, uuid::Uuid::new_v4().simple())
    }

    /// Process one provisioning request against the registered configs.
    pub fn provision(
        &mut self,
        registry: &mut DeviceRegistry,
        req: &ProvisionRequest,
    ) -> ProvisionResponse {
        let Some(cfg) = self.configs.get(&req.provision_device_key).cloned() else {
            return ProvisionResponse::NotFound;
        };
        if cfg.provision_device_secret != req.provision_device_secret {
            return ProvisionResponse::NotFound;
        }
        match cfg.strategy {
            ProvisionStrategy::Disabled => ProvisionResponse::Failure {
                reason: "provisioning disabled for device profile".into(),
            },
            ProvisionStrategy::CheckPreProvisionedDevices => {
                // Device must already exist in the tenant.
                let existing = registry
                    .devices_of_tenant(&cfg.tenant_id)
                    .find(|d| d.name == req.device_name)
                    .map(|d| d.id.clone());
                match existing {
                    Some(id) => self.issue(registry, id),
                    None => ProvisionResponse::NotFound,
                }
            }
            ProvisionStrategy::AllowCreateNewDevices => {
                // Reuse an existing same-name device, else create one.
                let existing = registry
                    .devices_of_tenant(&cfg.tenant_id)
                    .find(|d| d.name == req.device_name)
                    .map(|d| d.id.clone());
                let device_id = match existing {
                    Some(id) => id,
                    None => match registry.create_device(
                        &cfg.tenant_id,
                        &req.device_name,
                        &cfg.device_profile_id,
                        None,
                    ) {
                        Ok(d) => d.id,
                        Err(e) => {
                            return ProvisionResponse::Failure { reason: e.to_string() }
                        }
                    },
                };
                self.issue(registry, device_id)
            }
        }
    }

    /// Mint + attach an access token to a device and return Success.
    fn issue(&mut self, registry: &mut DeviceRegistry, device_id: String) -> ProvisionResponse {
        let token = self.next_token();
        match registry.set_credentials(DeviceCredentials {
            device_id: device_id.clone(),
            credentials_type: CredentialsType::AccessToken,
            credentials_id: token.clone(),
            credentials_value: None,
        }) {
            Ok(()) => ProvisionResponse::Success {
                device_id,
                access_token: token,
            },
            Err(e) => ProvisionResponse::Failure { reason: e.to_string() },
        }
    }

    /// Provision many devices by name under one config. Bad names (empty /
    /// duplicate) are counted as failures rather than aborting the batch.
    pub fn bulk_provision(
        &mut self,
        registry: &mut DeviceRegistry,
        key: &str,
        secret: &str,
        names: &[String],
    ) -> BulkReport {
        let mut created = 0;
        let mut failed = 0;
        for name in names {
            let resp = self.provision(
                registry,
                &ProvisionRequest {
                    device_name: name.clone(),
                    provision_device_key: key.to_string(),
                    provision_device_secret: secret.to_string(),
                },
            );
            match resp {
                ProvisionResponse::Success { .. } => created += 1,
                _ => failed += 1,
            }
        }
        BulkReport { created, failed }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::DeviceProfile;

    fn setup() -> (DeviceRegistry, ProvisionService, String) {
        let mut reg = DeviceRegistry::new();
        let p = DeviceProfile::new("prof", "t1", "sensors", TransportType::Mqtt);
        let pid = reg.save_profile(p).unwrap();
        (reg, ProvisionService::new(), pid)
    }

    #[test]
    fn allow_create_provisions_a_new_device_with_token() {
        let (mut reg, mut svc, pid) = setup();
        svc.register_config(ProvisionConfig {
            tenant_id: "t1".into(),
            device_profile_id: pid.clone(),
            strategy: ProvisionStrategy::AllowCreateNewDevices,
            provision_device_key: "KEY".into(),
            provision_device_secret: "SEC".into(),
        });
        let resp = svc.provision(
            &mut reg,
            &ProvisionRequest {
                device_name: "new-dev".into(),
                provision_device_key: "KEY".into(),
                provision_device_secret: "SEC".into(),
            },
        );
        let (device_id, token) = match resp {
            ProvisionResponse::Success { device_id, access_token } => (device_id, access_token),
            other => panic!("expected success, got {other:?}"),
        };
        // The device now exists and the issued token authenticates it.
        assert_eq!(reg.get_device(&device_id).unwrap().name, "new-dev");
        assert_eq!(reg.authenticate(&token).unwrap().id, device_id);
    }

    #[test]
    fn wrong_secret_yields_not_found() {
        let (mut reg, mut svc, pid) = setup();
        svc.register_config(ProvisionConfig {
            tenant_id: "t1".into(),
            device_profile_id: pid,
            strategy: ProvisionStrategy::AllowCreateNewDevices,
            provision_device_key: "KEY".into(),
            provision_device_secret: "SEC".into(),
        });
        let resp = svc.provision(
            &mut reg,
            &ProvisionRequest {
                device_name: "x".into(),
                provision_device_key: "KEY".into(),
                provision_device_secret: "WRONG".into(),
            },
        );
        assert_eq!(resp, ProvisionResponse::NotFound);
    }

    #[test]
    fn disabled_strategy_returns_failure() {
        let (mut reg, mut svc, pid) = setup();
        svc.register_config(ProvisionConfig {
            tenant_id: "t1".into(),
            device_profile_id: pid,
            strategy: ProvisionStrategy::Disabled,
            provision_device_key: "KEY".into(),
            provision_device_secret: "SEC".into(),
        });
        let resp = svc.provision(
            &mut reg,
            &ProvisionRequest {
                device_name: "x".into(),
                provision_device_key: "KEY".into(),
                provision_device_secret: "SEC".into(),
            },
        );
        assert!(matches!(resp, ProvisionResponse::Failure { .. }));
    }

    #[test]
    fn check_pre_provisioned_requires_existing_device() {
        let (mut reg, mut svc, pid) = setup();
        svc.register_config(ProvisionConfig {
            tenant_id: "t1".into(),
            device_profile_id: pid.clone(),
            strategy: ProvisionStrategy::CheckPreProvisionedDevices,
            provision_device_key: "KEY".into(),
            provision_device_secret: "SEC".into(),
        });
        let req = ProvisionRequest {
            device_name: "pre".into(),
            provision_device_key: "KEY".into(),
            provision_device_secret: "SEC".into(),
        };
        // No such device yet → NOT_FOUND.
        assert_eq!(svc.provision(&mut reg, &req), ProvisionResponse::NotFound);
        // Pre-provision it, then the same request succeeds.
        reg.create_device("t1", "pre", &pid, None).unwrap();
        assert!(matches!(
            svc.provision(&mut reg, &req),
            ProvisionResponse::Success { .. }
        ));
    }

    #[test]
    fn re_provisioning_existing_device_in_allow_mode_reuses_device() {
        let (mut reg, mut svc, pid) = setup();
        svc.register_config(ProvisionConfig {
            tenant_id: "t1".into(),
            device_profile_id: pid.clone(),
            strategy: ProvisionStrategy::AllowCreateNewDevices,
            provision_device_key: "KEY".into(),
            provision_device_secret: "SEC".into(),
        });
        let existing = reg.create_device("t1", "dup", &pid, None).unwrap();
        let resp = svc.provision(
            &mut reg,
            &ProvisionRequest {
                device_name: "dup".into(),
                provision_device_key: "KEY".into(),
                provision_device_secret: "SEC".into(),
            },
        );
        match resp {
            ProvisionResponse::Success { device_id, .. } => assert_eq!(device_id, existing.id),
            other => panic!("expected reuse, got {other:?}"),
        }
    }

    #[test]
    fn bulk_provision_creates_many_and_skips_bad_rows() {
        let (mut reg, mut svc, pid) = setup();
        svc.register_config(ProvisionConfig {
            tenant_id: "t1".into(),
            device_profile_id: pid,
            strategy: ProvisionStrategy::AllowCreateNewDevices,
            provision_device_key: "KEY".into(),
            provision_device_secret: "SEC".into(),
        });
        let names = vec!["a".to_string(), "b".to_string(), "".to_string()];
        let report = svc.bulk_provision(&mut reg, "KEY", "SEC", &names);
        assert_eq!(report.created, 2);
        assert_eq!(report.failed, 1);
    }
}

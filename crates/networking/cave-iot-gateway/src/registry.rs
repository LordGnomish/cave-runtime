// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Device registry — ThingsBoard `Device` / `DeviceProfile` /
//! `DeviceCredentials` domain (dao/device + dao/deviceprofile).
//!
//! Models the control-plane entities and an in-memory store enforcing the
//! ThingsBoard invariants: device names are unique per tenant, every device
//! references a device profile, and credentials provide an O(1) reverse
//! lookup from an access token to the owning device (the hot path of MQTT /
//! HTTP / CoAP authentication).

use crate::{IotError, Result};
use std::collections::HashMap;

/// Transport bound to a device profile (ThingsBoard `DeviceTransportType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum TransportType {
    Default,
    Mqtt,
    Coap,
    Lwm2m,
    Snmp,
}

/// A device profile: the shared template (transport + provisioning policy)
/// that many devices inherit from (`DeviceProfile`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DeviceProfile {
    pub id: String,
    pub tenant_id: String,
    pub name: String,
    pub transport_type: TransportType,
    /// Exactly one profile per tenant may be the default.
    pub is_default: bool,
}

/// Credential kind (`DeviceCredentialsType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CredentialsType {
    AccessToken,
    X509Certificate,
    MqttBasic,
    Lwm2mCredentials,
}

/// Device credentials (`DeviceCredentials`). `credentials_id` is the indexed
/// lookup key (access token, or the SHA of an X.509 cert).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DeviceCredentials {
    pub device_id: String,
    pub credentials_type: CredentialsType,
    pub credentials_id: String,
    pub credentials_value: Option<String>,
}

/// A device (`Device`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Device {
    pub id: String,
    pub tenant_id: String,
    pub name: String,
    pub device_profile_id: String,
    pub label: Option<String>,
    pub device_type: String,
}

/// In-memory device registry with the ThingsBoard uniqueness invariants.
#[derive(Debug, Default)]
pub struct DeviceRegistry {
    devices: HashMap<String, Device>,
    profiles: HashMap<String, DeviceProfile>,
    /// device_id → credentials
    credentials: HashMap<String, DeviceCredentials>,
    /// credentials_id → device_id (reverse auth index)
    cred_index: HashMap<String, String>,
}

impl DeviceProfile {
    pub fn new(id: &str, tenant_id: &str, name: &str, transport: TransportType) -> Self {
        DeviceProfile {
            id: id.to_string(),
            tenant_id: tenant_id.to_string(),
            name: name.to_string(),
            transport_type: transport,
            is_default: false,
        }
    }
}

impl DeviceRegistry {
    pub fn new() -> Self {
        Self::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(reg: &mut DeviceRegistry, tenant: &str) -> String {
        let p = DeviceProfile::new("p1", tenant, "default", TransportType::Default);
        reg.save_profile(p).unwrap()
    }

    #[test]
    fn create_device_assigns_id_and_is_retrievable() {
        let mut reg = DeviceRegistry::new();
        let pid = profile(&mut reg, "t1");
        let dev = reg
            .create_device("t1", "thermostat-1", &pid, Some("Lobby"))
            .unwrap();
        assert!(!dev.id.is_empty());
        assert_eq!(reg.get_device(&dev.id).unwrap().name, "thermostat-1");
        assert_eq!(reg.get_device(&dev.id).unwrap().label.as_deref(), Some("Lobby"));
    }

    #[test]
    fn duplicate_name_in_same_tenant_is_rejected() {
        let mut reg = DeviceRegistry::new();
        let pid = profile(&mut reg, "t1");
        reg.create_device("t1", "dup", &pid, None).unwrap();
        let err = reg.create_device("t1", "dup", &pid, None).unwrap_err();
        assert!(matches!(err, IotError::Invalid(_)));
    }

    #[test]
    fn same_name_in_different_tenants_is_allowed() {
        let mut reg = DeviceRegistry::new();
        let p1 = profile(&mut reg, "t1");
        let p2 = profile(&mut reg, "t2");
        reg.create_device("t1", "shared", &p1, None).unwrap();
        assert!(reg.create_device("t2", "shared", &p2, None).is_ok());
    }

    #[test]
    fn create_device_with_unknown_profile_fails() {
        let mut reg = DeviceRegistry::new();
        let err = reg.create_device("t1", "x", "nonexistent", None).unwrap_err();
        assert!(matches!(err, IotError::NotFound(_)));
    }

    #[test]
    fn access_token_reverse_lookup_authenticates_device() {
        let mut reg = DeviceRegistry::new();
        let pid = profile(&mut reg, "t1");
        let dev = reg.create_device("t1", "sensor", &pid, None).unwrap();
        reg.set_credentials(DeviceCredentials {
            device_id: dev.id.clone(),
            credentials_type: CredentialsType::AccessToken,
            credentials_id: "SECRET_TOKEN".into(),
            credentials_value: None,
        })
        .unwrap();
        let found = reg.authenticate("SECRET_TOKEN").unwrap();
        assert_eq!(found.id, dev.id);
        assert!(reg.authenticate("WRONG").is_err());
    }

    #[test]
    fn deleting_device_removes_credential_index() {
        let mut reg = DeviceRegistry::new();
        let pid = profile(&mut reg, "t1");
        let dev = reg.create_device("t1", "sensor", &pid, None).unwrap();
        reg.set_credentials(DeviceCredentials {
            device_id: dev.id.clone(),
            credentials_type: CredentialsType::AccessToken,
            credentials_id: "TOK".into(),
            credentials_value: None,
        })
        .unwrap();
        reg.delete_device(&dev.id).unwrap();
        assert!(reg.get_device(&dev.id).is_err());
        assert!(reg.authenticate("TOK").is_err());
    }

    #[test]
    fn rotating_credentials_drops_the_old_token() {
        let mut reg = DeviceRegistry::new();
        let pid = profile(&mut reg, "t1");
        let dev = reg.create_device("t1", "sensor", &pid, None).unwrap();
        reg.set_credentials(DeviceCredentials {
            device_id: dev.id.clone(),
            credentials_type: CredentialsType::AccessToken,
            credentials_id: "OLD".into(),
            credentials_value: None,
        })
        .unwrap();
        reg.set_credentials(DeviceCredentials {
            device_id: dev.id.clone(),
            credentials_type: CredentialsType::AccessToken,
            credentials_id: "NEW".into(),
            credentials_value: None,
        })
        .unwrap();
        assert!(reg.authenticate("OLD").is_err());
        assert_eq!(reg.authenticate("NEW").unwrap().id, dev.id);
    }

    #[test]
    fn default_profile_is_unique_per_tenant() {
        let mut reg = DeviceRegistry::new();
        let mut p1 = DeviceProfile::new("d1", "t1", "A", TransportType::Default);
        p1.is_default = true;
        reg.save_profile(p1).unwrap();
        let mut p2 = DeviceProfile::new("d2", "t1", "B", TransportType::Default);
        p2.is_default = true;
        // Second default in the same tenant must be rejected.
        assert!(reg.save_profile(p2).is_err());
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! # cave-iot-gateway
//!
//! Pure-Rust IoT device-management gateway modelled on
//! [ThingsBoard](https://github.com/thingsboard/thingsboard) v4.3.1.2
//! (Apache-2.0) with an Eclipse Kura (EPL) telemetry-codec compatibility
//! layer.
//!
//! The crate ports the *control-plane domain logic* of an IoT platform in
//! pure Rust with no live sockets or radios:
//!
//! - **Device registry & provisioning** ([`registry`], [`provisioning`]) —
//!   devices, device profiles, credentials, claim + bulk provisioning.
//! - **Multi-protocol telemetry ingestion** ([`transport`]) — MQTT topic +
//!   packet codec, HTTP device-API mapping, CoAP (RFC 7252) message codec,
//!   LoRaWAN Cayenne-LPP uplink decode, Modbus PDU framing.
//! - **Rule engine** ([`rule_engine`]) — filter / transform / action node
//!   chains with message routing.
//! - **Device twin** ([`twin`]) — attribute scopes + desired/reported delta.
//! - **OTA campaigns** ([`ota`]) — firmware packages + rollout state machine.
//! - **Time-series storage** ([`timeseries`]) — ts-kv entries + aggregation.
//! - **Multi-tenancy** ([`tenant`]) — tenant isolation + entity quotas.
//!
//! Live transport listeners (MQTT broker TCP/TLS, CoAP/DTLS socket, the
//! LoRaWAN network-server join procedure, OSGi/Kura bundle runtime) are
//! deliberately out of scope — see `parity.manifest.toml` `[[skipped]]`.

use std::collections::BTreeMap;

pub mod ota;
pub mod provisioning;
pub mod registry;
pub mod rule_engine;
pub mod tenant;
pub mod timeseries;
pub mod transport;
pub mod twin;

/// A telemetry / attribute scalar value (ThingsBoard `KvEntry` data types).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
pub enum KvValue {
    Bool(bool),
    Long(i64),
    Double(f64),
    Str(String),
    Json(serde_json::Value),
}

impl KvValue {
    /// Coerce to `f64` when the value is numeric (Long/Double) — used by
    /// time-series aggregation and rule-engine numeric filters.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            KvValue::Long(v) => Some(*v as f64),
            KvValue::Double(v) => Some(*v),
            KvValue::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
            _ => None,
        }
    }
}

/// A bag of key→value telemetry/attribute pairs.
pub type KvMap = BTreeMap<String, KvValue>;

/// Crate-wide error type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IotError {
    /// Entity not found in a registry/store.
    NotFound(String),
    /// Validation failed (bad name, duplicate, malformed credential, …).
    Invalid(String),
    /// A protocol codec rejected a frame.
    Codec(String),
    /// A tenant isolation / quota rule was violated.
    TenantViolation(String),
    /// A state-machine transition was illegal.
    IllegalTransition(String),
}

impl std::fmt::Display for IotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IotError::NotFound(s) => write!(f, "not found: {s}"),
            IotError::Invalid(s) => write!(f, "invalid: {s}"),
            IotError::Codec(s) => write!(f, "codec error: {s}"),
            IotError::TenantViolation(s) => write!(f, "tenant violation: {s}"),
            IotError::IllegalTransition(s) => write!(f, "illegal transition: {s}"),
        }
    }
}

impl std::error::Error for IotError {}

/// Crate result alias.
pub type Result<T> = std::result::Result<T, IotError>;

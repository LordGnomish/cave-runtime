// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Multi-protocol telemetry ingestion codecs.
//!
//! Each submodule is a *pure* codec — it parses and builds wire frames /
//! payloads but opens no sockets. Live listeners (MQTT broker, CoAP/DTLS
//! server, LoRaWAN network server) are runtime data-plane components and are
//! out of scope (see `parity.manifest.toml`).

pub mod http;
pub mod mqtt;

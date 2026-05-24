// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Event-export layer — JSON NDJSON stream + gRPC framing codec.
//!
//! Upstream: `pkg/exporter/exporter.go`, `pkg/encoder/json_encoder.go`.

pub mod grpc_codec;
pub mod json_stream;

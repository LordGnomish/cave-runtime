// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! RunFunctionRequest / RunFunctionResponse wire shape + JSON codec.
//!
//! Upstream: proto/fn/v1/run_function.proto
//!
//! cave-crossplane exchanges these JSON-encoded over the in-process bus; gRPC
//! wire encoding is a Phase 2 concern routed via cave-llm-gateway. We provide
//! the shape + bytes codec so external sidecars can speak the same model.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunFunctionRequest {
    /// Logging metadata — request id, target XR ref, etc.
    pub meta: RequestMeta,
    /// Free-form input for the function (specific to each function type).
    pub input: Value,
    /// Observed state from the XR + composed resources.
    pub observed: Value,
    /// Optional extra resources resolved from `requirements`.
    #[serde(default)]
    pub extra_resources: Value,
    /// Context passed through across pipeline steps.
    pub context: String,
}

impl RunFunctionRequest {
    pub fn new(context: impl Into<String>, input: Value, observed: Value) -> Self {
        Self {
            meta: RequestMeta::default(),
            input,
            observed,
            extra_resources: Value::Null,
            context: context.into(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RequestMeta {
    pub tag: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunFunctionResponse {
    pub meta: ResponseMeta,
    /// Desired state for the XR + composed resources.
    pub desired: Value,
    /// Observed state passed through (may be modified by the function).
    pub observed: Value,
    /// Optional context shared across steps.
    pub context: Value,
    /// Step results (severity strings: "NORMAL" / "WARNING" / "FATAL").
    #[serde(default)]
    pub results: Vec<String>,
    /// Optional requirement requests (additional resources to fetch next iter).
    #[serde(default)]
    pub requirements: Vec<Requirement>,
    /// TTL in seconds the orchestrator should treat output as fresh.
    #[serde(default)]
    pub ttl_seconds: u64,
}

impl RunFunctionResponse {
    pub fn ready(desired: Value) -> Self {
        Self {
            meta: ResponseMeta::default(),
            desired,
            observed: Value::Null,
            context: Value::Null,
            results: Vec::new(),
            requirements: Vec::new(),
            ttl_seconds: 60,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResponseMeta {
    pub tag: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Requirement {
    pub name: String,
    pub api_version: String,
    pub kind: String,
    #[serde(default)]
    pub match_labels: std::collections::BTreeMap<String, String>,
}

/// Encode a request to JSON bytes.
pub fn encode_request(req: &RunFunctionRequest) -> Vec<u8> {
    serde_json::to_vec(req).unwrap_or_default()
}

/// Decode a response from JSON bytes.
pub fn decode_response(bytes: &[u8]) -> Result<RunFunctionResponse, String> {
    serde_json::from_slice(bytes).map_err(|e| e.to_string())
}

/// Encode a response.
pub fn encode_response(resp: &RunFunctionResponse) -> Vec<u8> {
    serde_json::to_vec(resp).unwrap_or_default()
}

/// Decode a request.
pub fn decode_request(bytes: &[u8]) -> Result<RunFunctionRequest, String> {
    serde_json::from_slice(bytes).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn request_round_trip() {
        let r = RunFunctionRequest::new("ctx", json!({"k":"v"}), json!({}));
        let b = encode_request(&r);
        let back = decode_request(&b).unwrap();
        assert_eq!(back.context, "ctx");
        assert_eq!(back.input["k"], json!("v"));
    }

    #[test]
    fn response_round_trip() {
        let r = RunFunctionResponse::ready(json!({"d":1}));
        let b = encode_response(&r);
        let back = decode_response(&b).unwrap();
        assert_eq!(back.desired["d"], json!(1));
        assert_eq!(back.ttl_seconds, 60);
    }

    #[test]
    fn decode_invalid_errors() {
        assert!(decode_response(b"not json").is_err());
    }

    #[test]
    fn requirements_default_empty() {
        let r = RunFunctionResponse::ready(json!({}));
        assert!(r.requirements.is_empty());
    }

    #[test]
    fn results_default_empty() {
        let r = RunFunctionResponse::ready(json!({}));
        assert!(r.results.is_empty());
    }

    #[test]
    fn requirement_with_labels() {
        let mut req = Requirement {
            name: "n".into(),
            api_version: "v1".into(),
            kind: "ConfigMap".into(),
            match_labels: Default::default(),
        };
        req.match_labels.insert("env".into(), "prod".into());
        let s = serde_json::to_string(&req).unwrap();
        assert!(s.contains("prod"));
    }

    #[test]
    fn meta_tags_default_empty() {
        let r = RunFunctionRequest::new("c", json!({}), json!({}));
        assert!(r.meta.tag.is_empty());
    }
}

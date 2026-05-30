// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Backend orchestration glue — OpenJarvis primitive.
//!
//! A single switchable interface over the local inference backends Cave
//! already ports: `cave-local-llm` (Ollama + vLLM engine internals),
//! `cave-mlx` (Apple Silicon array core), and the in-process Hermes
//! runtime itself. The orchestration layer decides *which* backend serves
//! a request — local-first by default (cloud endpoints are filtered out
//! unless explicitly allowed) and ranked by caller-supplied priority.

use serde::{Deserialize, Serialize};

use crate::error::HermesError;

/// The switchable inference backends Cave ports locally.
///
/// All four run on-device — `Ollama` and `Vllm` via `cave-local-llm`,
/// `Mlx` via `cave-mlx`, and `Hermes` being the in-process orchestration
/// runtime acting as its own trivial backend (canned/echo responses for
/// deterministic flows that need no model).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Backend {
    Ollama,
    Vllm,
    Mlx,
    Hermes,
}

impl Backend {
    pub fn as_str(&self) -> &'static str {
        match self {
            Backend::Ollama => "ollama",
            Backend::Vllm => "vllm",
            Backend::Mlx => "mlx",
            Backend::Hermes => "hermes",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "ollama" => Some(Backend::Ollama),
            "vllm" => Some(Backend::Vllm),
            "mlx" => Some(Backend::Mlx),
            "hermes" => Some(Backend::Hermes),
            _ => None,
        }
    }

    /// The device a backend prefers when the caller gives no hint.
    pub fn default_device(&self) -> Device {
        match self {
            Backend::Mlx => Device::Metal,
            Backend::Vllm => Device::Cuda,
            Backend::Ollama => Device::Auto,
            Backend::Hermes => Device::Cpu,
        }
    }
}

impl std::fmt::Display for Backend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Compute device a backend runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Device {
    Cpu,
    Cuda,
    Metal,
    Auto,
}

/// Where a backend is reachable. Local-first orchestration only selects
/// `Local` endpoints unless [`BackendRegistry::allow_remote`] is set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Endpoint {
    Local,
    Remote(String),
}

impl Endpoint {
    pub fn is_local(&self) -> bool {
        matches!(self, Endpoint::Local)
    }
}

/// One registered backend option.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackendProfile {
    pub backend: Backend,
    pub model: String,
    pub device: Device,
    pub endpoint: Endpoint,
    /// Higher wins when [`BackendRegistry::select`] ranks candidates.
    pub priority: i32,
}

impl BackendProfile {
    pub fn new(backend: Backend, model: impl Into<String>) -> Self {
        Self {
            backend,
            model: model.into(),
            device: backend.default_device(),
            endpoint: Endpoint::Local,
            priority: 0,
        }
    }

    pub fn device(mut self, device: Device) -> Self {
        self.device = device;
        self
    }

    pub fn endpoint(mut self, endpoint: Endpoint) -> Self {
        self.endpoint = endpoint;
        self
    }

    pub fn priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }
}

/// Switchable registry of backend options. Local-first by default.
#[derive(Debug, Default)]
pub struct BackendRegistry {
    profiles: Vec<BackendProfile>,
    local_first: bool,
}

impl BackendRegistry {
    pub fn new() -> Self {
        Self {
            profiles: Vec::new(),
            local_first: true,
        }
    }

    pub fn local_first(&self) -> bool {
        self.local_first
    }

    /// Allow (or forbid) remote endpoints. Forbidden by default.
    pub fn allow_remote(&mut self, allow: bool) -> &mut Self {
        self.local_first = !allow;
        self
    }

    pub fn register(&mut self, profile: BackendProfile) -> &mut Self {
        self.profiles.push(profile);
        self
    }

    /// All profiles selectable under the current locality policy.
    pub fn candidates(&self) -> Vec<&BackendProfile> {
        self.profiles
            .iter()
            .filter(|p| !self.local_first || p.endpoint.is_local())
            .collect()
    }

    /// Highest-priority selectable backend, or [`HermesError::Backend`] if
    /// none are selectable under the locality policy.
    pub fn select(&self) -> crate::error::Result<&BackendProfile> {
        self.candidates()
            .into_iter()
            .max_by_key(|p| p.priority)
            .ok_or_else(|| {
                HermesError::Backend(if self.local_first {
                    "no local backend registered (local-first mode)".into()
                } else {
                    "no backend registered".into()
                })
            })
    }

    /// First selectable profile for a named backend, if any.
    pub fn select_for(&self, backend: Backend) -> Option<&BackendProfile> {
        self.candidates()
            .into_iter()
            .find(|p| p.backend == backend)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_str_roundtrip() {
        for b in [Backend::Ollama, Backend::Vllm, Backend::Mlx, Backend::Hermes] {
            assert_eq!(Backend::from_name(b.as_str()), Some(b));
        }
        assert_eq!(Backend::from_name("nope"), None);
    }

    #[test]
    fn local_endpoint_is_local_remote_is_not() {
        assert!(Endpoint::Local.is_local());
        assert!(!Endpoint::Remote("https://api.example.com".into()).is_local());
    }

    #[test]
    fn registry_is_local_first_by_default() {
        let r = BackendRegistry::new();
        assert!(r.local_first(), "OpenJarvis is local-first by default");
    }

    #[test]
    fn local_first_filters_out_remote_candidates() {
        let mut r = BackendRegistry::new();
        r.register(
            BackendProfile::new(Backend::Ollama, "qwen3")
                .endpoint(Endpoint::Local)
                .priority(10),
        );
        r.register(
            BackendProfile::new(Backend::Vllm, "llama-cloud")
                .endpoint(Endpoint::Remote("https://vllm.example.com".into()))
                .priority(99),
        );
        let cands = r.candidates();
        assert_eq!(cands.len(), 1, "remote backend must be hidden in local-first mode");
        assert_eq!(cands[0].backend, Backend::Ollama);
    }

    #[test]
    fn allow_remote_unhides_remote_candidates() {
        let mut r = BackendRegistry::new();
        r.allow_remote(true);
        r.register(
            BackendProfile::new(Backend::Vllm, "llama-cloud")
                .endpoint(Endpoint::Remote("https://vllm.example.com".into())),
        );
        assert_eq!(r.candidates().len(), 1);
    }

    #[test]
    fn select_picks_highest_priority_local_backend() {
        let mut r = BackendRegistry::new();
        r.register(BackendProfile::new(Backend::Ollama, "qwen3").priority(5));
        r.register(BackendProfile::new(Backend::Mlx, "mlx-7b").priority(20));
        let pick = r.select().unwrap();
        assert_eq!(pick.backend, Backend::Mlx);
    }

    #[test]
    fn select_errors_when_no_local_candidate() {
        let mut r = BackendRegistry::new();
        r.register(
            BackendProfile::new(Backend::Vllm, "remote-only")
                .endpoint(Endpoint::Remote("https://x".into())),
        );
        let err = r.select().unwrap_err();
        assert!(matches!(err, crate::error::HermesError::Backend(_)));
    }

    #[test]
    fn select_for_returns_named_backend() {
        let mut r = BackendRegistry::new();
        r.register(BackendProfile::new(Backend::Ollama, "qwen3"));
        r.register(BackendProfile::new(Backend::Mlx, "mlx-7b"));
        let p = r.select_for(Backend::Mlx).unwrap();
        assert_eq!(p.model, "mlx-7b");
        assert!(r.select_for(Backend::Hermes).is_none());
    }

    #[test]
    fn default_device_matches_backend_locality() {
        // MLX is Apple-Silicon Metal; vLLM defaults to CUDA; Ollama auto.
        assert_eq!(Backend::Mlx.default_device(), Device::Metal);
        assert_eq!(Backend::Vllm.default_device(), Device::Cuda);
        assert_eq!(Backend::Ollama.default_device(), Device::Auto);
    }
}

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

// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use crate::models::{CloudProvider, PricingConfig};
use std::collections::HashMap;
use uuid::Uuid;

/// Default pricing rates for common cloud providers.
/// Returns `(cpu_core_hour, memory_gb_hour, storage_gb_month, network_egress_gb, gpu_core_hour)`.
pub fn default_rates(provider: &CloudProvider) -> (f64, f64, f64, f64, f64) {
    match provider {
        CloudProvider::Aws => (0.048, 0.006, 0.10, 0.09, 2.48),
        CloudProvider::Gcp => (0.040, 0.005, 0.08, 0.08, 2.20),
        CloudProvider::Azure => (0.045, 0.006, 0.095, 0.087, 2.35),
        CloudProvider::OnPrem => (0.020, 0.003, 0.05, 0.01, 1.50),
        CloudProvider::Custom => (0.0, 0.0, 0.0, 0.0, 0.0),
    }
}

/// Create a PricingConfig with default rates for the given provider.
pub fn default_config_for_provider(name: &str, provider: CloudProvider) -> PricingConfig {
    let (cpu, mem, stor, net, gpu) = default_rates(&provider);
    PricingConfig {
        id: Uuid::new_v4(),
        name: name.to_string(),
        provider,
        cpu_core_hour: cpu,
        memory_gb_hour: mem,
        storage_gb_month: stor,
        network_egress_gb: net,
        gpu_core_hour: gpu,
        custom_rates: HashMap::new(),
        created_at: chrono::Utc::now(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_aws_rates() {
        let (cpu, mem, stor, net, gpu) = default_rates(&CloudProvider::Aws);
        assert!(cpu > 0.0);
        assert!(mem > 0.0);
        assert!(stor > 0.0);
        assert!(net > 0.0);
        assert!(gpu > 0.0);
    }

    #[test]
    fn test_custom_rates_zero() {
        let (cpu, mem, stor, net, gpu) = default_rates(&CloudProvider::Custom);
        assert_eq!(cpu, 0.0);
        assert_eq!(mem, 0.0);
        assert_eq!(stor, 0.0);
        assert_eq!(net, 0.0);
        assert_eq!(gpu, 0.0);
    }

    #[test]
    fn test_default_config() {
        let cfg = default_config_for_provider("test-aws", CloudProvider::Aws);
        assert_eq!(cfg.name, "test-aws");
        assert!(cfg.cpu_core_hour > 0.0);
    }

    #[test]
    fn test_all_providers_have_rates() {
        for provider in [
            CloudProvider::Aws,
            CloudProvider::Gcp,
            CloudProvider::Azure,
            CloudProvider::OnPrem,
        ] {
            let (cpu, mem, _, _, _) = default_rates(&provider);
            assert!(cpu > 0.0, "Expected positive CPU rate for provider");
            assert!(mem > 0.0, "Expected positive memory rate for provider");
        }
    }
}

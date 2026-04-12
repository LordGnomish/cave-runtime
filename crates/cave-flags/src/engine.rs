//! Flag evaluation engine — determines if a flag is enabled for a given context.

use crate::models::{EvaluationContext, FeatureFlag, FlagEvaluation, Strategy};
use std::hash::{DefaultHasher, Hash, Hasher};

/// Evaluate all flags for a given context.
pub fn evaluate_flags(flags: &[FeatureFlag], context: &EvaluationContext) -> Vec<FlagEvaluation> {
    flags
        .iter()
        .map(|flag| evaluate_single(flag, context))
        .collect()
}

/// Evaluate a single flag against a context.
pub fn evaluate_single(flag: &FeatureFlag, context: &EvaluationContext) -> FlagEvaluation {
    // Kill switch always wins
    if flag.kill_switch {
        return FlagEvaluation {
            name: flag.name.clone(),
            enabled: false,
            variant: None,
        };
    }

    // Check environment scope
    if !flag.environments.is_empty() && !flag.environments.contains(&context.environment) {
        return FlagEvaluation {
            name: flag.name.clone(),
            enabled: false,
            variant: None,
        };
    }

    // Check tenant scope
    if let Some(ref flag_tenant) = flag.tenant_id {
        if context.tenant_id.as_ref() != Some(flag_tenant) {
            return FlagEvaluation {
                name: flag.name.clone(),
                enabled: false,
                variant: None,
            };
        }
    }

    // Global enable check
    if !flag.enabled {
        return FlagEvaluation {
            name: flag.name.clone(),
            enabled: false,
            variant: None,
        };
    }

    // Evaluate strategy
    let enabled = match &flag.strategy {
        Strategy::Default { enabled } => *enabled,
        Strategy::GradualRollout {
            percentage,
            group_id,
        } => {
            let key = format!(
                "{}:{}",
                group_id.as_deref().unwrap_or(&flag.name),
                context.user_id.as_deref().unwrap_or("anonymous")
            );
            normalize_hash(&key) < *percentage as u32
        }
        Strategy::UserIds { user_ids } => context
            .user_id
            .as_ref()
            .map_or(false, |uid| user_ids.contains(uid)),
        Strategy::TenantScope { tenant_ids } => context
            .tenant_id
            .as_ref()
            .map_or(false, |tid| tenant_ids.contains(tid)),
        Strategy::EnvironmentScope { environments } => {
            environments.contains(&context.environment)
        }
        Strategy::Custom { .. } => {
            // Custom strategies evaluated via plugin system (future)
            flag.enabled
        }
    };

    FlagEvaluation {
        name: flag.name.clone(),
        enabled,
        variant: None, // TODO: variant support in next iteration
    }
}

/// Normalize a string to a 0-99 hash for percentage rollout.
/// Uses Murmur3-like distribution for even spread.
fn normalize_hash(key: &str) -> u32 {
    let mut hasher = DefaultHasher::new();
    key.hash(&mut hasher);
    (hasher.finish() % 100) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::*;
    use chrono::Utc;
    use uuid::Uuid;

    fn make_flag(name: &str, strategy: Strategy) -> FeatureFlag {
        FeatureFlag {
            id: Uuid::new_v4(),
            name: name.to_string(),
            description: String::new(),
            enabled: true,
            flag_type: FlagType::Boolean,
            strategy,
            environments: vec![],
            tenant_id: None,
            kill_switch: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            created_by: Uuid::new_v4(),
        }
    }

    fn make_context(env: &str) -> EvaluationContext {
        EvaluationContext {
            user_id: Some("user-123".to_string()),
            tenant_id: Some("tenant-acme".to_string()),
            environment: env.to_string(),
            properties: None,
        }
    }

    #[test]
    fn test_default_strategy_enabled() {
        let flag = make_flag("test", Strategy::Default { enabled: true });
        let result = evaluate_single(&flag, &make_context("prod"));
        assert!(result.enabled);
    }

    #[test]
    fn test_kill_switch_overrides() {
        let mut flag = make_flag("test", Strategy::Default { enabled: true });
        flag.kill_switch = true;
        let result = evaluate_single(&flag, &make_context("prod"));
        assert!(!result.enabled);
    }

    #[test]
    fn test_environment_scope() {
        let mut flag = make_flag("test", Strategy::Default { enabled: true });
        flag.environments = vec!["staging".to_string()];
        let result = evaluate_single(&flag, &make_context("prod"));
        assert!(!result.enabled);
    }

    #[test]
    fn test_user_id_strategy() {
        let flag = make_flag(
            "test",
            Strategy::UserIds {
                user_ids: vec!["user-123".to_string()],
            },
        );
        let result = evaluate_single(&flag, &make_context("prod"));
        assert!(result.enabled);
    }

    #[test]
    fn test_gradual_rollout_deterministic() {
        let flag = make_flag(
            "test",
            Strategy::GradualRollout {
                percentage: 50,
                group_id: None,
            },
        );
        let ctx = make_context("prod");
        let r1 = evaluate_single(&flag, &ctx);
        let r2 = evaluate_single(&flag, &ctx);
        assert_eq!(r1.enabled, r2.enabled); // Same user = same result
    }

    #[test]
    fn test_disabled_flag_not_evaluated() {
        let mut flag = make_flag("disabled-flag", Strategy::Default { enabled: true });
        flag.enabled = false;
        let result = evaluate_single(&flag, &make_context("prod"));
        assert!(!result.enabled);
    }

    #[test]
    fn test_tenant_scope_mismatch() {
        let mut flag = make_flag("tenant-flag", Strategy::Default { enabled: true });
        flag.tenant_id = Some("tenant-a".to_string());
        let ctx = EvaluationContext {
            user_id: Some("user-1".to_string()),
            tenant_id: Some("tenant-b".to_string()),
            environment: "prod".to_string(),
            properties: None,
        };
        let result = evaluate_single(&flag, &ctx);
        assert!(!result.enabled);
    }

    #[test]
    fn test_tenant_scope_match() {
        let mut flag = make_flag("tenant-flag", Strategy::Default { enabled: true });
        flag.tenant_id = Some("tenant-a".to_string());
        let ctx = EvaluationContext {
            user_id: Some("user-1".to_string()),
            tenant_id: Some("tenant-a".to_string()),
            environment: "prod".to_string(),
            properties: None,
        };
        let result = evaluate_single(&flag, &ctx);
        assert!(result.enabled);
    }

    #[test]
    fn test_user_ids_no_match() {
        let flag = make_flag(
            "user-flag",
            Strategy::UserIds {
                user_ids: vec!["user-abc".to_string(), "user-xyz".to_string()],
            },
        );
        let ctx = EvaluationContext {
            user_id: Some("user-not-in-list".to_string()),
            tenant_id: None,
            environment: "prod".to_string(),
            properties: None,
        };
        let result = evaluate_single(&flag, &ctx);
        assert!(!result.enabled);
    }

    #[test]
    fn test_evaluate_flags_batch() {
        let flags = vec![
            make_flag("flag-1", Strategy::Default { enabled: true }),
            make_flag("flag-2", Strategy::Default { enabled: false }),
            make_flag("flag-3", Strategy::Default { enabled: true }),
        ];
        let ctx = make_context("prod");
        let results = evaluate_flags(&flags, &ctx);
        assert_eq!(results.len(), 3);
        assert!(results[0].enabled);
        assert!(!results[1].enabled);
        assert!(results[2].enabled);
    }

    #[test]
    fn test_gradual_rollout_zero_percent() {
        let flag = make_flag(
            "rollout-zero",
            Strategy::GradualRollout {
                percentage: 0,
                group_id: None,
            },
        );
        // normalize_hash returns 0-99, so 0 > any hash value is never true
        // We test a few different users
        for user in &["user-a", "user-b", "user-c", "user-d", "user-e"] {
            let ctx = EvaluationContext {
                user_id: Some(user.to_string()),
                tenant_id: None,
                environment: "prod".to_string(),
                properties: None,
            };
            let result = evaluate_single(&flag, &ctx);
            assert!(!result.enabled, "Expected disabled for user {}", user);
        }
    }

    #[test]
    fn test_gradual_rollout_hundred_percent() {
        let flag = make_flag(
            "rollout-hundred",
            Strategy::GradualRollout {
                percentage: 100,
                group_id: None,
            },
        );
        // normalize_hash returns 0-99, so hash < 100 is always true
        for user in &["user-a", "user-b", "user-c", "user-d", "user-e"] {
            let ctx = EvaluationContext {
                user_id: Some(user.to_string()),
                tenant_id: None,
                environment: "prod".to_string(),
                properties: None,
            };
            let result = evaluate_single(&flag, &ctx);
            assert!(result.enabled, "Expected enabled for user {}", user);
        }
    }

    #[test]
    fn test_environment_scope_strategy() {
        let flag = make_flag(
            "env-scope-flag",
            Strategy::EnvironmentScope {
                environments: vec!["staging".to_string(), "prod".to_string()],
            },
        );
        let prod_ctx = make_context("prod");
        let dev_ctx = make_context("dev");
        assert!(evaluate_single(&flag, &prod_ctx).enabled);
        assert!(!evaluate_single(&flag, &dev_ctx).enabled);
    }

    #[test]
    fn test_custom_strategy_uses_flag_enabled() {
        let flag = make_flag(
            "custom-flag",
            Strategy::Custom {
                name: "my-plugin".to_string(),
                parameters: serde_json::json!({"key": "value"}),
            },
        );
        // Custom strategy falls back to flag.enabled (which is true in make_flag)
        let result = evaluate_single(&flag, &make_context("prod"));
        assert!(result.enabled);

        // Now test with a disabled flag using custom strategy
        let mut disabled_flag = make_flag(
            "custom-disabled",
            Strategy::Custom {
                name: "my-plugin".to_string(),
                parameters: serde_json::json!({}),
            },
        );
        disabled_flag.enabled = false;
        let result2 = evaluate_single(&disabled_flag, &make_context("prod"));
        assert!(!result2.enabled);
    }

    #[test]
    fn test_normalize_hash_bounded() {
        // We test via the public evaluate_single with GradualRollout which uses normalize_hash internally.
        // Since normalize_hash is private, we verify its behavior indirectly: 100% rollout always passes,
        // 0% rollout always fails, confirming hash is in [0, 99].
        let keys = vec![
            "user-alpha",
            "user-beta",
            "user-gamma",
            "",
            "very-long-user-key-that-exceeds-normal-length-12345",
            "🦀",
        ];
        for key in keys {
            let ctx = EvaluationContext {
                user_id: Some(key.to_string()),
                tenant_id: None,
                environment: "prod".to_string(),
                properties: None,
            };
            let flag_zero = make_flag(
                "hash-test-zero",
                Strategy::GradualRollout { percentage: 0, group_id: None },
            );
            let flag_hundred = make_flag(
                "hash-test-hundred",
                Strategy::GradualRollout { percentage: 100, group_id: None },
            );
            assert!(!evaluate_single(&flag_zero, &ctx).enabled, "0% should always be false for key '{}'", key);
            assert!(evaluate_single(&flag_hundred, &ctx).enabled, "100% should always be true for key '{}'", key);
        }
    }
}

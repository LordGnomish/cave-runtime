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
}

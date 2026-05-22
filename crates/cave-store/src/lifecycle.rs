// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use crate::types::{LifecycleRule, StorageClass};

pub struct LifecycleManager;

impl LifecycleManager {
    /// Returns true if the object should be expired based on the rule.
    pub fn should_expire(rule: &LifecycleRule, object_age_days: u32) -> bool {
        if !rule.enabled {
            return false;
        }
        if let Some(expiration_days) = rule.expiration_days {
            return object_age_days >= expiration_days;
        }
        false
    }

    /// Returns the storage class to transition to, if applicable.
    pub fn should_transition(rule: &LifecycleRule, object_age_days: u32) -> Option<StorageClass> {
        if !rule.enabled {
            return None;
        }
        if let Some(transition_days) = rule.transition_days {
            if object_age_days >= transition_days {
                return rule.transition_storage_class.clone();
            }
        }
        None
    }

    /// Evaluate rules for a list of (key, age_days) pairs.
    pub fn evaluate_rules(
        rules: &[LifecycleRule],
        objects: &[(String, u32)],
    ) -> Vec<(String, LifecycleAction)> {
        let mut actions = Vec::new();
        for (key, age_days) in objects {
            for rule in rules {
                if !rule.enabled {
                    continue;
                }
                // Check prefix
                if !key.starts_with(&rule.prefix) {
                    continue;
                }
                if Self::should_expire(rule, *age_days) {
                    actions.push((key.clone(), LifecycleAction::Expire));
                    break;
                }
                if let Some(storage_class) = Self::should_transition(rule, *age_days) {
                    actions.push((key.clone(), LifecycleAction::Transition(storage_class)));
                    break;
                }
            }
        }
        actions
    }
}

pub enum LifecycleAction {
    Expire,
    Transition(StorageClass),
}

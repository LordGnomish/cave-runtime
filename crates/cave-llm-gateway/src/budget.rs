// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Spend-budget tracking — direct port of LiteLLM `litellm/budget_manager.py`.
//!
//! LiteLLM's `BudgetManager` tracks per-user USD spend against an allocated
//! `total_budget`, with an optional rolling reset window (`daily`/`weekly`/
//! `monthly`/`yearly`). The gateway wires a [`BudgetManager`] into
//! [`crate::router::GatewayRouter`] so the live `complete()` pipeline rejects a
//! request with [`crate::error::GatewayError::BudgetExceeded`] once a consumer
//! has spent past their limit (LiteLLM raises `BudgetExceededError`).

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Rolling reset window for a budget — mirrors LiteLLM's `duration` literals
/// (`daily`/`weekly`/`monthly`/`yearly`) which map to fixed day counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BudgetDuration {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

impl BudgetDuration {
    /// Day count per LiteLLM (`daily=1, weekly=7, monthly=30, yearly=365`).
    pub fn days(&self) -> i64 {
        match self {
            Self::Daily => 1,
            Self::Weekly => 7,
            Self::Monthly => 30,
            Self::Yearly => 365,
        }
    }

    pub fn seconds(&self) -> i64 {
        self.days() * 86_400
    }
}

/// One consumer's budget ledger entry (LiteLLM `user_dict[user]`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserBudget {
    pub user: String,
    pub total_budget: f64,
    pub current_cost: f64,
    /// Per-model spend breakdown (LiteLLM `model_cost`).
    pub model_cost: HashMap<String, f64>,
    pub duration: Option<BudgetDuration>,
    pub created_at: DateTime<Utc>,
    pub last_updated_at: DateTime<Utc>,
}

/// Thread-safe spend-budget manager — port of `litellm.BudgetManager`.
#[derive(Default)]
pub struct BudgetManager {
    users: DashMap<String, UserBudget>,
}

impl BudgetManager {
    pub fn new() -> Self {
        Self {
            users: DashMap::new(),
        }
    }

    /// Allocate `total_budget` USD to `user`, optionally with a rolling
    /// reset `duration`. Re-creating an existing user resets their ledger.
    pub fn create_budget(&self, total_budget: f64, user: &str, duration: Option<BudgetDuration>) {
        let now = Utc::now();
        self.users.insert(
            user.to_string(),
            UserBudget {
                user: user.to_string(),
                total_budget,
                current_cost: 0.0,
                model_cost: HashMap::new(),
                duration,
                created_at: now,
                last_updated_at: now,
            },
        );
    }

    /// Record `cost` USD spent by `user` on `model`, updating both the
    /// aggregate `current_cost` and the per-model breakdown. No-op when the
    /// user has no budget configured (LiteLLM only tracks created users).
    pub fn update_cost(&self, user: &str, model: &str, cost: f64) {
        if let Some(mut b) = self.users.get_mut(user) {
            b.current_cost += cost;
            *b.model_cost.entry(model.to_string()).or_insert(0.0) += cost;
            b.last_updated_at = Utc::now();
        }
    }

    /// Accumulated spend for `user` (0.0 if untracked).
    pub fn get_current_cost(&self, user: &str) -> f64 {
        self.users.get(user).map(|b| b.current_cost).unwrap_or(0.0)
    }

    /// Per-model spend for `user` (0.0 if untracked).
    pub fn model_cost(&self, user: &str, model: &str) -> f64 {
        self.users
            .get(user)
            .and_then(|b| b.model_cost.get(model).copied())
            .unwrap_or(0.0)
    }

    /// Allocated budget for `user`, or `None` when no budget is configured.
    pub fn get_total_budget(&self, user: &str) -> Option<f64> {
        self.users.get(user).map(|b| b.total_budget)
    }

    /// Existing spend plus a hypothetical `additional_cost` (LiteLLM
    /// `projected_cost`, which tokenizes the next request first).
    pub fn projected_cost(&self, user: &str, additional_cost: f64) -> f64 {
        self.get_current_cost(user) + additional_cost
    }

    /// Remaining budget for `user`, or `None` when no budget is configured.
    pub fn remaining(&self, user: &str) -> Option<f64> {
        self.users.get(user).map(|b| b.total_budget - b.current_cost)
    }

    /// `true` when the user is within budget. An unconfigured user is
    /// unlimited (LiteLLM only enforces limits on tracked users).
    pub fn is_within_budget(&self, user: &str) -> bool {
        match self.users.get(user) {
            Some(b) => b.current_cost <= b.total_budget,
            None => true,
        }
    }

    /// Clear aggregate and per-model spend for `user`, keeping the budget.
    pub fn reset_cost(&self, user: &str) {
        if let Some(mut b) = self.users.get_mut(user) {
            b.current_cost = 0.0;
            b.model_cost.clear();
            b.last_updated_at = Utc::now();
        }
    }

    /// Reset spend if the rolling window elapsed since `created_at`
    /// (LiteLLM `reset_on_duration`). Returns `true` when a reset fired.
    pub fn reset_on_duration(&self, user: &str) -> bool {
        let should_reset = {
            match self.users.get(user) {
                Some(b) => match b.duration {
                    Some(d) => {
                        Utc::now().signed_duration_since(b.created_at).num_seconds() >= d.seconds()
                    }
                    None => false,
                },
                None => false,
            }
        };
        if should_reset {
            if let Some(mut b) = self.users.get_mut(user) {
                b.current_cost = 0.0;
                b.model_cost.clear();
                let now = Utc::now();
                b.created_at = now;
                b.last_updated_at = now;
            }
        }
        should_reset
    }

    /// `true` when `user` has a budget entry.
    pub fn is_valid_user(&self, user: &str) -> bool {
        self.users.contains_key(user)
    }

    /// All tracked consumer identifiers.
    pub fn get_users(&self) -> Vec<String> {
        self.users.iter().map(|e| e.key().clone()).collect()
    }

    /// Snapshot of `user`'s ledger entry.
    pub fn snapshot(&self, user: &str) -> Option<UserBudget> {
        self.users.get(user).map(|b| b.clone())
    }

    /// Snapshot of every tracked ledger entry.
    pub fn list(&self) -> Vec<UserBudget> {
        self.users.iter().map(|e| e.value().clone()).collect()
    }

    /// Test-only: rewind a user's `created_at`/`last_updated_at` so the
    /// rolling-window reset path is exercisable without sleeping.
    #[cfg(test)]
    pub fn backdate(&self, user: &str, by: chrono::Duration) {
        if let Some(mut b) = self.users.get_mut(user) {
            b.created_at -= by;
            b.last_updated_at -= by;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn create_and_query_budget() {
        let m = BudgetManager::new();
        m.create_budget(100.0, "alice", Some(BudgetDuration::Monthly));
        assert_eq!(m.get_total_budget("alice"), Some(100.0));
        assert_eq!(m.get_current_cost("alice"), 0.0);
        assert!(m.is_valid_user("alice"));
        assert!(!m.is_valid_user("bob"));
    }

    #[test]
    fn update_cost_accumulates_aggregate_and_per_model() {
        let m = BudgetManager::new();
        m.create_budget(100.0, "alice", None);
        m.update_cost("alice", "gpt-4o", 1.5);
        m.update_cost("alice", "gpt-4o", 0.5);
        m.update_cost("alice", "claude-sonnet-4-6", 2.0);
        assert!((m.get_current_cost("alice") - 4.0).abs() < 1e-9);
        assert!((m.model_cost("alice", "gpt-4o") - 2.0).abs() < 1e-9);
        assert!((m.model_cost("alice", "claude-sonnet-4-6") - 2.0).abs() < 1e-9);
    }

    #[test]
    fn within_budget_until_exceeded() {
        let m = BudgetManager::new();
        m.create_budget(5.0, "alice", None);
        assert!(m.is_within_budget("alice"));
        m.update_cost("alice", "gpt-4o", 4.99);
        assert!(m.is_within_budget("alice"));
        m.update_cost("alice", "gpt-4o", 0.02); // 5.01 > 5.0
        assert!(!m.is_within_budget("alice"));
    }

    #[test]
    fn no_budget_configured_is_unlimited() {
        let m = BudgetManager::new();
        assert!(m.is_within_budget("nobody"));
        assert_eq!(m.get_total_budget("nobody"), None);
        assert_eq!(m.remaining("nobody"), None);
    }

    #[test]
    fn projected_cost_adds_to_current() {
        let m = BudgetManager::new();
        m.create_budget(100.0, "alice", None);
        m.update_cost("alice", "gpt-4o", 10.0);
        assert!((m.projected_cost("alice", 2.5) - 12.5).abs() < 1e-9);
    }

    #[test]
    fn reset_cost_clears_aggregate_and_model() {
        let m = BudgetManager::new();
        m.create_budget(100.0, "alice", None);
        m.update_cost("alice", "gpt-4o", 10.0);
        m.reset_cost("alice");
        assert_eq!(m.get_current_cost("alice"), 0.0);
        assert!(m.model_cost("alice", "gpt-4o").abs() < 1e-9);
    }

    #[test]
    fn reset_on_duration_resets_after_elapsed_window() {
        let m = BudgetManager::new();
        m.create_budget(100.0, "alice", Some(BudgetDuration::Daily));
        m.update_cost("alice", "gpt-4o", 10.0);
        // Force created/last-updated 2 days into the past.
        m.backdate("alice", Duration::days(2));
        assert!(m.reset_on_duration("alice"));
        assert_eq!(m.get_current_cost("alice"), 0.0);
    }

    #[test]
    fn reset_on_duration_noop_within_window() {
        let m = BudgetManager::new();
        m.create_budget(100.0, "alice", Some(BudgetDuration::Monthly));
        m.update_cost("alice", "gpt-4o", 10.0);
        assert!(!m.reset_on_duration("alice"));
        assert!((m.get_current_cost("alice") - 10.0).abs() < 1e-9);
    }

    #[test]
    fn reset_on_duration_noop_without_duration() {
        let m = BudgetManager::new();
        m.create_budget(100.0, "alice", None);
        m.update_cost("alice", "gpt-4o", 10.0);
        m.backdate("alice", Duration::days(9999));
        assert!(!m.reset_on_duration("alice"));
        assert!((m.get_current_cost("alice") - 10.0).abs() < 1e-9);
    }

    #[test]
    fn get_users_lists_all_tracked() {
        let m = BudgetManager::new();
        m.create_budget(10.0, "alice", None);
        m.create_budget(20.0, "bob", None);
        let mut users = m.get_users();
        users.sort();
        assert_eq!(users, vec!["alice".to_string(), "bob".to_string()]);
    }

    #[test]
    fn duration_maps_to_litellm_day_counts() {
        assert_eq!(BudgetDuration::Daily.days(), 1);
        assert_eq!(BudgetDuration::Weekly.days(), 7);
        assert_eq!(BudgetDuration::Monthly.days(), 30);
        assert_eq!(BudgetDuration::Yearly.days(), 365);
    }

    #[test]
    fn remaining_budget_reported() {
        let m = BudgetManager::new();
        m.create_budget(100.0, "alice", None);
        m.update_cost("alice", "gpt-4o", 30.0);
        assert!((m.remaining("alice").unwrap() - 70.0).abs() < 1e-9);
    }
}

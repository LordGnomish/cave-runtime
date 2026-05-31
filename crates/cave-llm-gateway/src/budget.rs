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

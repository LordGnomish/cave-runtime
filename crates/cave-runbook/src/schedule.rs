// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Scheduled and alert-triggered execution.
use crate::models::{RunbookTrigger, TriggerType};
use std::collections::HashMap;
use uuid::Uuid;

pub fn create_schedule_trigger(
    runbook_id: Uuid,
    cron_expression: &str,
    parameters: HashMap<String, serde_json::Value>,
) -> RunbookTrigger {
    RunbookTrigger {
        id: Uuid::new_v4(),
        runbook_id,
        trigger_type: TriggerType::Scheduled,
        cron_expression: Some(cron_expression.to_string()),
        alert_source: None,
        alert_condition: None,
        parameters,
        enabled: true,
        last_triggered_at: None,
        created_at: chrono::Utc::now(),
    }
}

pub fn create_alert_trigger(
    runbook_id: Uuid,
    alert_source: &str,
    alert_condition: &str,
    parameters: HashMap<String, serde_json::Value>,
) -> RunbookTrigger {
    RunbookTrigger {
        id: Uuid::new_v4(),
        runbook_id,
        trigger_type: TriggerType::Alert,
        cron_expression: None,
        alert_source: Some(alert_source.to_string()),
        alert_condition: Some(alert_condition.to_string()),
        parameters,
        enabled: true,
        last_triggered_at: None,
        created_at: chrono::Utc::now(),
    }
}

pub fn validate_cron(expr: &str) -> bool {
    expr.split_whitespace().count() == 5
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_schedule_trigger() {
        let id = Uuid::new_v4();
        let trigger = create_schedule_trigger(id, "0 * * * *", HashMap::new());
        assert_eq!(trigger.trigger_type, TriggerType::Scheduled);
        assert!(trigger.enabled);
    }

    #[test]
    fn test_validate_cron() {
        assert!(validate_cron("0 * * * *"));
        assert!(!validate_cron("invalid"));
    }
}

//! Log-based alerting rules.
//!
//! Rules are stored in-process (Arc<RwLock<Vec<AlertRule>>>).  The evaluator
//! runs periodically and fires alerts when the LogQL metric expression crosses
//! the configured threshold.

use crate::models::{AlertRule, FiredAlert};
use crate::store::LogStore;
use chrono::Utc;
use std::sync::{Arc, RwLock};
use tracing::{debug, info, warn};

pub struct AlertManager {
    rules: Arc<RwLock<Vec<AlertRule>>>,
    store: Arc<LogStore>,
}

impl AlertManager {
    pub fn new(store: Arc<LogStore>) -> Self {
        Self {
            rules: Arc::new(RwLock::new(Vec::new())),
            store,
        }
    }

    pub fn add_rule(&self, rule: AlertRule) {
        self.rules.write().unwrap().push(rule);
    }

    pub fn remove_rule(&self, id: uuid::Uuid) {
        self.rules.write().unwrap().retain(|r| r.id != id);
    }

    pub fn list_rules(&self) -> Vec<AlertRule> {
        self.rules.read().unwrap().clone()
    }

    /// Evaluate all rules against the current store state.
    /// Returns any fired alerts.
    pub fn evaluate(&self) -> Vec<FiredAlert> {
        let rules = self.rules.read().unwrap().clone();
        let now = Utc::now();
        let mut fired = vec![];

        for rule in &rules {
            debug!(rule = %rule.name, "evaluating alert rule");
            match self.store.eval_alert(rule, now) {
                Some(alert) => {
                    info!(
                        rule = %alert.rule_name,
                        value = alert.value,
                        severity = ?alert.severity,
                        "alert fired"
                    );
                    fired.push(alert);
                }
                None => {}
            }
        }

        fired
    }

    /// Background task: evaluate rules every `interval_secs` seconds.
    pub async fn run_loop(self: Arc<Self>, interval_secs: u64) {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
        loop {
            ticker.tick().await;
            let fired = self.evaluate();
            if !fired.is_empty() {
                warn!(count = fired.len(), "alert rules fired");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{AlertCondition, AlertSeverity, CompareOp, Labels, LogEntry};
    use chrono::{Duration, Utc};
    use std::collections::HashMap;

    fn make_store() -> Arc<LogStore> {
        Arc::new(LogStore::new(Duration::days(1)))
    }

    #[test]
    fn alert_fires_when_count_exceeds_threshold() {
        let store = make_store();
        let labels = Labels::new([("app".into(), "failing".into())].into());
        for _ in 0..10 {
            store.push(
                labels.clone(),
                vec![LogEntry {
                    timestamp: Utc::now() - Duration::seconds(30),
                    line: "ERROR something went wrong".into(),
                    structured_metadata: HashMap::new(),
                }],
                None,
            );
        }

        let manager = AlertManager::new(Arc::clone(&store));
        manager.add_rule(AlertRule {
            id: uuid::Uuid::new_v4(),
            name: "high-error-rate".into(),
            expr: r#"count_over_time({app="failing"}[5m])"#.into(),
            duration_secs: 300,
            condition: AlertCondition { op: CompareOp::Gt, threshold: 5.0 },
            severity: AlertSeverity::Critical,
            annotations: HashMap::new(),
            tenant: None,
        });

        let fired = manager.evaluate();
        assert!(!fired.is_empty(), "alert should have fired");
        assert!(fired[0].value > 5.0);
    }

    #[test]
    fn alert_does_not_fire_below_threshold() {
        let store = make_store();
        let labels = Labels::new([("app".into(), "quiet".into())].into());
        store.push(
            labels,
            vec![LogEntry {
                timestamp: Utc::now() - Duration::seconds(30),
                line: "INFO ok".into(),
                structured_metadata: HashMap::new(),
            }],
            None,
        );

        let manager = AlertManager::new(Arc::clone(&store));
        manager.add_rule(AlertRule {
            id: uuid::Uuid::new_v4(),
            name: "high-count".into(),
            expr: r#"count_over_time({app="quiet"}[5m])"#.into(),
            duration_secs: 300,
            condition: AlertCondition { op: CompareOp::Gt, threshold: 100.0 },
            severity: AlertSeverity::Warning,
            annotations: HashMap::new(),
            tenant: None,
        });

        let fired = manager.evaluate();
        assert!(fired.is_empty(), "alert should not have fired");
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Experiment CRD parity — `argoproj/argo-rollouts v1.9.0`
//! (`pkg/apis/rollouts/v1alpha1/experiment_types.go`).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ExperimentTemplate {
    pub name: String,
    pub replicas: u32,
    pub image: String,
    #[serde(default)]
    pub selector: Option<String>,
    #[serde(default)]
    pub weight: Option<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ExperimentAnalysis {
    pub template_name: String,
    pub inconclusive_limit: u32,
    pub failure_limit: u32,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum ExperimentPhase {
    Pending,
    Running,
    Successful,
    Failed,
    Inconclusive,
    Error,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Experiment {
    pub id: Uuid,
    pub name: String,
    pub namespace: String,
    pub duration_seconds: u64,
    pub templates: Vec<ExperimentTemplate>,
    #[serde(default)]
    pub analyses: Vec<ExperimentAnalysis>,
    pub phase: ExperimentPhase,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub message: Option<String>,
}

impl Experiment {
    pub fn new(
        name: impl Into<String>,
        namespace: impl Into<String>,
        duration_seconds: u64,
        templates: Vec<ExperimentTemplate>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            namespace: namespace.into(),
            duration_seconds,
            templates,
            analyses: Vec::new(),
            phase: ExperimentPhase::Pending,
            started_at: None,
            finished_at: None,
            message: None,
        }
    }

    pub fn start(&mut self, now: DateTime<Utc>) {
        if matches!(self.phase, ExperimentPhase::Pending) {
            self.phase = ExperimentPhase::Running;
            self.started_at = Some(now);
        }
    }

    pub fn evaluate(
        &mut self,
        failures: u32,
        inconclusives: u32,
        now: DateTime<Utc>,
    ) -> ExperimentPhase {
        if !matches!(self.phase, ExperimentPhase::Running) {
            return self.phase;
        }
        let any_breaches_failure = self.analyses.iter().any(|a| failures > a.failure_limit);
        let any_breaches_inconclusive = self
            .analyses
            .iter()
            .any(|a| inconclusives > a.inconclusive_limit);
        let next = if any_breaches_failure {
            ExperimentPhase::Failed
        } else if any_breaches_inconclusive {
            ExperimentPhase::Inconclusive
        } else if let Some(start) = self.started_at {
            let elapsed = (now - start).num_seconds().max(0) as u64;
            if elapsed >= self.duration_seconds {
                ExperimentPhase::Successful
            } else {
                ExperimentPhase::Running
            }
        } else {
            ExperimentPhase::Running
        };
        if next != ExperimentPhase::Running {
            self.phase = next;
            self.finished_at = Some(now);
        }
        self.phase
    }

    pub fn abort(&mut self, reason: impl Into<String>, now: DateTime<Utc>) {
        if matches!(self.phase, ExperimentPhase::Pending | ExperimentPhase::Running) {
            self.phase = ExperimentPhase::Failed;
            self.message = Some(reason.into());
            self.finished_at = Some(now);
        }
    }

    pub fn total_replicas(&self) -> u32 {
        self.templates.iter().map(|t| t.replicas).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tpl(name: &str, replicas: u32) -> ExperimentTemplate {
        ExperimentTemplate {
            name: name.into(),
            replicas,
            image: format!("ghcr.io/cave/{name}:v1"),
            selector: None,
            weight: None,
        }
    }

    #[test]
    fn new_experiment_starts_pending() {
        let e = Experiment::new("e1", "argo", 60, vec![tpl("a", 1)]);
        assert!(matches!(e.phase, ExperimentPhase::Pending));
        assert!(e.started_at.is_none());
    }

    #[test]
    fn start_is_idempotent_from_running() {
        let mut e = Experiment::new("e", "n", 60, vec![tpl("a", 1)]);
        let t = Utc::now();
        e.start(t);
        e.start(t + chrono::Duration::seconds(1));
        assert_eq!(e.started_at, Some(t));
    }

    #[test]
    fn evaluate_promotes_to_successful_after_duration() {
        let mut e = Experiment::new("e", "n", 10, vec![tpl("a", 1)]);
        let t0 = Utc::now();
        e.start(t0);
        let phase = e.evaluate(0, 0, t0 + chrono::Duration::seconds(15));
        assert_eq!(phase, ExperimentPhase::Successful);
        assert!(e.finished_at.is_some());
    }

    #[test]
    fn evaluate_holds_running_while_under_duration() {
        let mut e = Experiment::new("e", "n", 60, vec![tpl("a", 1)]);
        let t0 = Utc::now();
        e.start(t0);
        let phase = e.evaluate(0, 0, t0 + chrono::Duration::seconds(5));
        assert_eq!(phase, ExperimentPhase::Running);
    }

    #[test]
    fn evaluate_transitions_to_failed_on_failure_breach() {
        let mut e = Experiment::new("e", "n", 60, vec![tpl("a", 1)]);
        e.analyses.push(ExperimentAnalysis {
            template_name: "latency".into(),
            inconclusive_limit: 5,
            failure_limit: 2,
        });
        let t0 = Utc::now();
        e.start(t0);
        let phase = e.evaluate(3, 0, t0 + chrono::Duration::seconds(1));
        assert_eq!(phase, ExperimentPhase::Failed);
    }

    #[test]
    fn evaluate_transitions_to_inconclusive_when_caps_breached() {
        let mut e = Experiment::new("e", "n", 60, vec![tpl("a", 1)]);
        e.analyses.push(ExperimentAnalysis {
            template_name: "latency".into(),
            inconclusive_limit: 1,
            failure_limit: 5,
        });
        let t0 = Utc::now();
        e.start(t0);
        let phase = e.evaluate(0, 2, t0 + chrono::Duration::seconds(1));
        assert_eq!(phase, ExperimentPhase::Inconclusive);
    }

    #[test]
    fn abort_marks_failed_with_reason() {
        let mut e = Experiment::new("e", "n", 60, vec![tpl("a", 1)]);
        e.abort("stop", Utc::now());
        assert!(matches!(e.phase, ExperimentPhase::Failed));
        assert_eq!(e.message.as_deref(), Some("stop"));
    }

    #[test]
    fn abort_is_noop_on_finished_experiment() {
        let mut e = Experiment::new("e", "n", 1, vec![tpl("a", 1)]);
        let t0 = Utc::now();
        e.start(t0);
        let _ = e.evaluate(0, 0, t0 + chrono::Duration::seconds(5));
        let final_phase = e.phase;
        e.abort("late", t0 + chrono::Duration::seconds(10));
        assert_eq!(e.phase, final_phase);
    }

    #[test]
    fn total_replicas_sums_templates() {
        let e = Experiment::new("e", "n", 60, vec![tpl("a", 2), tpl("b", 3), tpl("c", 1)]);
        assert_eq!(e.total_replicas(), 6);
    }

    #[test]
    fn experiment_roundtrips_through_serde() {
        let mut e = Experiment::new("e", "n", 60, vec![tpl("a", 1)]);
        e.analyses.push(ExperimentAnalysis {
            template_name: "x".into(),
            inconclusive_limit: 1,
            failure_limit: 1,
        });
        let j = serde_json::to_string(&e).unwrap();
        let back: Experiment = serde_json::from_str(&j).unwrap();
        assert_eq!(back.templates.len(), 1);
        assert_eq!(back.analyses.len(), 1);
    }
}

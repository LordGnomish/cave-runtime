// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Prometheus metrics + health endpoint.
//!
//! Each instance binds `:<metrics_port>` (9101 for cave-runtime, 9102 for
//! cave-home) and serves two routes:
//!
//! * `GET /metrics`  — Prometheus text exposition of daemon state.
//! * `GET /healthz`  — JSON liveness: is the daemon ticking, when did it last
//!   work a task, how deep is the queue.
//!
//! [`MetricsSnapshot`] is the single source of truth; the daemon mutates it
//! under a mutex and the HTTP handlers render a clone. Rendering is pure and
//! unit-tested; only [`serve`] does I/O.

use axum::{extract::State, response::IntoResponse, routing::get, Router};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

/// Live daemon state surfaced to operators.
#[derive(Debug, Clone)]
pub struct MetricsSnapshot {
    pub instance: String,
    pub uptime_secs: u64,
    pub tasks_completed: u64,
    pub tasks_failed: u64,
    pub tasks_escalated_human: u64,
    pub tasks_reached_claude: u64,
    /// Per-tier LLM call counts, keyed by [`Tier::label`](crate::escalation::Tier::label).
    pub llm_calls: BTreeMap<String, u64>,
    pub claude_tokens_used: u64,
    pub queue_depth: usize,
    pub current_task: Option<String>,
    pub mean_completion: f64,
    pub idle: bool,
    /// Unix seconds of the last task the daemon worked (0 if none yet).
    pub last_task_unix: u64,
}

impl MetricsSnapshot {
    pub fn new(instance: &str) -> Self {
        Self {
            instance: instance.to_string(),
            uptime_secs: 0,
            tasks_completed: 0,
            tasks_failed: 0,
            tasks_escalated_human: 0,
            tasks_reached_claude: 0,
            llm_calls: BTreeMap::new(),
            claude_tokens_used: 0,
            queue_depth: 0,
            current_task: None,
            mean_completion: 0.0,
            idle: false,
            last_task_unix: 0,
        }
    }

    /// Bump a per-tier LLM call counter.
    pub fn record_llm_call(&mut self, tier_label: &str) {
        *self.llm_calls.entry(tier_label.to_string()).or_insert(0) += 1;
    }

    /// Fraction of finished tasks that needed Claude escalation. Zero when
    /// nothing has finished yet.
    pub fn escalation_rate(&self) -> f64 {
        let finished = self.tasks_completed + self.tasks_failed;
        if finished == 0 {
            return 0.0;
        }
        self.tasks_reached_claude as f64 / finished as f64
    }

    /// Render the Prometheus text exposition format.
    pub fn render_prometheus(&self) -> String {
        let inst = &self.instance;
        let mut s = String::new();
        let mut g = |name: &str, help: &str, ty: &str, val: String| {
            s.push_str(&format!("# HELP {name} {help}\n# TYPE {name} {ty}\n"));
            s.push_str(&format!("{name}{{instance=\"{inst}\"}} {val}\n"));
        };
        g("cave_autopilot_uptime_seconds", "Daemon uptime in seconds", "gauge", self.uptime_secs.to_string());
        g("cave_autopilot_tasks_completed_total", "Tasks completed (merged)", "counter", self.tasks_completed.to_string());
        g("cave_autopilot_tasks_failed_total", "Tasks failed after all tiers", "counter", self.tasks_failed.to_string());
        g("cave_autopilot_tasks_escalated_human_total", "Tasks escalated to a human", "counter", self.tasks_escalated_human.to_string());
        g("cave_autopilot_claude_tokens_used_total", "Claude tokens consumed today", "counter", self.claude_tokens_used.to_string());
        g("cave_autopilot_queue_depth", "Pending tasks in the queue", "gauge", self.queue_depth.to_string());
        g("cave_autopilot_mean_completion_ratio", "Mean subsystem completion", "gauge", format!("{:.4}", self.mean_completion));
        g("cave_autopilot_escalation_rate", "Fraction of finished tasks that reached Claude", "gauge", format!("{:.4}", self.escalation_rate()));
        g("cave_autopilot_idle", "1 if daemon is in idle/monitor mode", "gauge", if self.idle { "1".into() } else { "0".into() });
        g("cave_autopilot_last_task_timestamp_seconds", "Unix time of last worked task", "gauge", self.last_task_unix.to_string());

        // Per-tier LLM call counts as a labelled family.
        s.push_str("# HELP cave_autopilot_llm_calls_total LLM calls per escalation tier\n");
        s.push_str("# TYPE cave_autopilot_llm_calls_total counter\n");
        for (tier, n) in &self.llm_calls {
            s.push_str(&format!(
                "cave_autopilot_llm_calls_total{{instance=\"{inst}\",tier=\"{tier}\"}} {n}\n"
            ));
        }
        s
    }

    /// Render the `/healthz` JSON body.
    pub fn render_health(&self) -> serde_json::Value {
        serde_json::json!({
            "status": "ok",
            "instance": self.instance,
            "uptime_secs": self.uptime_secs,
            "last_task_unix": self.last_task_unix,
            "queue_depth": self.queue_depth,
            "current_task": self.current_task,
            "idle": self.idle,
        })
    }
}

/// Shared, mutex-guarded snapshot handle the daemon updates and HTTP serves.
pub type SharedMetrics = Arc<Mutex<MetricsSnapshot>>;

/// Build the axum router for an already-shared snapshot.
pub fn router(shared: SharedMetrics) -> Router {
    Router::new()
        .route("/metrics", get(metrics_handler))
        .route("/healthz", get(health_handler))
        .with_state(shared)
}

async fn metrics_handler(State(m): State<SharedMetrics>) -> impl IntoResponse {
    let body = m.lock().expect("metrics mutex").render_prometheus();
    ([("content-type", "text/plain; version=0.0.4")], body)
}

async fn health_handler(State(m): State<SharedMetrics>) -> impl IntoResponse {
    let body = m.lock().expect("metrics mutex").render_health();
    axum::Json(body)
}

/// Bind `0.0.0.0:<port>` and serve metrics + health until the process exits.
pub async fn serve(port: u16, shared: SharedMetrics) -> crate::Result<()> {
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port))
        .await
        .map_err(|e| crate::AutopilotError::Config(format!("bind :{port}: {e}")))?;
    axum::serve(listener, router(shared))
        .await
        .map_err(|e| crate::AutopilotError::Config(format!("metrics serve: {e}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prometheus_has_help_type_and_values() {
        let mut m = MetricsSnapshot::new("cave-runtime");
        m.uptime_secs = 42;
        m.tasks_completed = 3;
        m.record_llm_call("l2_coder");
        m.record_llm_call("l2_coder");
        m.record_llm_call("l3_claude");
        let out = m.render_prometheus();
        assert!(out.contains("# TYPE cave_autopilot_uptime_seconds gauge"));
        assert!(out.contains("cave_autopilot_uptime_seconds{instance=\"cave-runtime\"} 42"));
        assert!(out.contains("cave_autopilot_tasks_completed_total{instance=\"cave-runtime\"} 3"));
        assert!(out.contains("cave_autopilot_llm_calls_total{instance=\"cave-runtime\",tier=\"l2_coder\"} 2"));
        assert!(out.contains("cave_autopilot_llm_calls_total{instance=\"cave-runtime\",tier=\"l3_claude\"} 1"));
    }

    #[test]
    fn escalation_rate_computes() {
        let mut m = MetricsSnapshot::new("x");
        m.tasks_completed = 8;
        m.tasks_failed = 2;
        m.tasks_reached_claude = 2;
        assert!((m.escalation_rate() - 0.2).abs() < 1e-9);
    }

    #[test]
    fn escalation_rate_zero_when_nothing_finished() {
        assert_eq!(MetricsSnapshot::new("x").escalation_rate(), 0.0);
    }

    #[test]
    fn health_json_carries_liveness_fields() {
        let mut m = MetricsSnapshot::new("cave-home");
        m.queue_depth = 7;
        m.current_task = Some("port-cave-etcd".into());
        let h = m.render_health();
        assert_eq!(h["status"], "ok");
        assert_eq!(h["instance"], "cave-home");
        assert_eq!(h["queue_depth"], 7);
        assert_eq!(h["current_task"], "port-cave-etcd");
    }
}

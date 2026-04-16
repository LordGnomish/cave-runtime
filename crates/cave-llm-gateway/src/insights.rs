//! High-level helpers that other CAVE crates use to drive observability and
//! self-healing flows through the gateway. Decouples obs/incidents/alerts
//! from any specific provider — they just call `InsightsEngine` and get
//! structured results back.
//!
//! All public methods are async and route through `GatewayRouter`, which
//! means provider choice (embedded vs. remote), caching, rate limiting,
//! cost tracking and guardrails are applied automatically.

use crate::error::{GatewayError, GatewayResult};
use crate::openai::{ChatCompletionRequest, ChatMessage};
use crate::router::GatewayRouter;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Caller used for logging/cost accounting on insight requests.
const CONSUMER: &str = "cave.insights";

pub struct InsightsEngine {
    router: Arc<GatewayRouter>,
    /// Model id (or alias) to route through. Typically "embedded" or an
    /// alias like "triage-fast" defined in `AliasRegistry`.
    model: String,
}

impl InsightsEngine {
    pub fn new(router: Arc<GatewayRouter>, model: impl Into<String>) -> Self {
        Self { router, model: model.into() }
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    // ── Generic chat helper ──────────────────────────────────────────────────

    async fn chat(&self, system: &str, user: &str, max_tokens: u32) -> GatewayResult<String> {
        let req = ChatCompletionRequest {
            model: self.model.clone(),
            messages: vec![ChatMessage::system(system), ChatMessage::user(user)],
            temperature: Some(0.2),
            top_p: Some(0.9),
            max_tokens: Some(max_tokens),
            stream: Some(false),
            stop: None,
            presence_penalty: None,
            frequency_penalty: None,
            n: None,
            user: Some(CONSUMER.into()),
            tools: None,
            tool_choice: None,
            response_format: None,
            seed: None,
            logprobs: None,
        };
        let resp = self.router.complete(CONSUMER, req).await?;
        let text = resp
            .choices
            .first()
            .and_then(|c| c.message.as_ref())
            .and_then(|m| m.content.as_text())
            .unwrap_or("")
            .trim()
            .to_string();
        if text.is_empty() {
            return Err(GatewayError::Internal("empty insight response".into()));
        }
        Ok(text)
    }

    async fn chat_json<T: for<'de> Deserialize<'de>>(
        &self,
        system: &str,
        user: &str,
        max_tokens: u32,
    ) -> GatewayResult<T> {
        let raw = self.chat(system, user, max_tokens).await?;
        let json = extract_json_block(&raw).unwrap_or(&raw);
        serde_json::from_str(json).map_err(|e| {
            GatewayError::Internal(format!("insight JSON parse failed: {e} — raw: {raw}"))
        })
    }

    // ── Incident summarisation ───────────────────────────────────────────────

    pub async fn summarize_incident(&self, ctx: &IncidentContext<'_>) -> GatewayResult<String> {
        let user = format!(
            "Incident: {title}\nSeverity: {sev}\nStatus: {status}\n\nDescription:\n{desc}\n\nTimeline:\n{timeline}\n\nWrite a 3-4 sentence executive summary. Lead with current state, then root cause if known, then next action.",
            title = ctx.title,
            sev = ctx.severity,
            status = ctx.status,
            desc = ctx.description,
            timeline = ctx
                .timeline
                .iter()
                .map(|e| format!("- [{}] {}: {}", e.timestamp, e.event_type, e.message))
                .collect::<Vec<_>>()
                .join("\n"),
        );
        self.chat(SYSTEM_INCIDENT_SUMMARY, &user, 400).await
    }

    // ── Log triage ───────────────────────────────────────────────────────────

    pub async fn triage_logs(&self, lines: &[String], window: &str) -> GatewayResult<LogTriage> {
        let joined = truncate_lines(lines, 200);
        let user = format!(
            "Window: {window}\nLog sample ({n} lines):\n{joined}\n\nReturn JSON with shape {{\"severity\":\"info|warn|error|critical\",\"summary\":\"...\",\"top_errors\":[\"...\"],\"recommended_actions\":[\"...\"]}}.",
            n = lines.len(),
        );
        self.chat_json(SYSTEM_LOG_TRIAGE, &user, 600).await
    }

    // ── Alert explanation ────────────────────────────────────────────────────

    pub async fn explain_alert(&self, ctx: &AlertContext<'_>) -> GatewayResult<String> {
        let labels = ctx
            .labels
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(", ");
        let user = format!(
            "Alert: {name}\nSeverity: {sev}\nLabels: {labels}\nValue: {val}\nQuery: {query}\n\nExplain in 2-3 sentences what this alert means in plain English and what a responder should check first.",
            name = ctx.name,
            sev = ctx.severity,
            val = ctx.value.unwrap_or(f64::NAN),
            query = ctx.query.unwrap_or(""),
        );
        self.chat(SYSTEM_ALERT_EXPLAIN, &user, 300).await
    }

    // ── Remediation proposal ────────────────────────────────────────────────

    pub async fn propose_remediation(
        &self,
        incident: &IncidentContext<'_>,
        relevant_logs: &[String],
    ) -> GatewayResult<RemediationPlan> {
        let logs = truncate_lines(relevant_logs, 80);
        let user = format!(
            "Incident: {title} (sev {sev})\nDescription: {desc}\n\nRelevant logs:\n{logs}\n\nPropose remediation as JSON: {{\"hypothesis\":\"...\",\"steps\":[{{\"description\":\"...\",\"action\":\"shell|http|k8s|manual\",\"command\":\"...\",\"risk\":\"low|medium|high\"}}],\"requires_approval\":true|false}}.",
            title = incident.title,
            sev = incident.severity,
            desc = incident.description,
        );
        self.chat_json(SYSTEM_REMEDIATION, &user, 800).await
    }

    // ── Metric anomaly ───────────────────────────────────────────────────────

    pub async fn explain_anomaly(&self, series: &MetricSeries<'_>) -> GatewayResult<AnomalyExplanation> {
        let samples = series
            .samples
            .iter()
            .map(|(t, v)| format!("{t},{v:.4}"))
            .collect::<Vec<_>>()
            .join("\n");
        let user = format!(
            "Metric: {name}\nLabels: {labels}\nSamples (ts,value):\n{samples}\n\nReturn JSON {{\"is_anomaly\":true|false,\"confidence\":0.0-1.0,\"pattern\":\"spike|drop|drift|seasonal|none\",\"explanation\":\"...\"}}.",
            name = series.name,
            labels = series
                .labels
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join(","),
        );
        self.chat_json(SYSTEM_ANOMALY, &user, 400).await
    }
}

// ── Input / output types ─────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct TimelineEntry<'a> {
    pub timestamp: &'a str,
    pub event_type: &'a str,
    pub message: &'a str,
}

#[derive(Debug, Clone)]
pub struct IncidentContext<'a> {
    pub title: &'a str,
    pub severity: &'a str,
    pub status: &'a str,
    pub description: &'a str,
    pub timeline: Vec<TimelineEntry<'a>>,
}

#[derive(Debug, Clone)]
pub struct AlertContext<'a> {
    pub name: &'a str,
    pub severity: &'a str,
    pub labels: Vec<(&'a str, &'a str)>,
    pub value: Option<f64>,
    pub query: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub struct MetricSeries<'a> {
    pub name: &'a str,
    pub labels: Vec<(&'a str, &'a str)>,
    pub samples: Vec<(i64, f64)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogTriage {
    pub severity: String,
    pub summary: String,
    #[serde(default)]
    pub top_errors: Vec<String>,
    #[serde(default)]
    pub recommended_actions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemediationStep {
    pub description: String,
    pub action: String,
    #[serde(default)]
    pub command: String,
    pub risk: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemediationPlan {
    pub hypothesis: String,
    pub steps: Vec<RemediationStep>,
    #[serde(default = "default_true")]
    pub requires_approval: bool,
}

fn default_true() -> bool { true }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnomalyExplanation {
    pub is_anomaly: bool,
    #[serde(default)]
    pub confidence: f32,
    pub pattern: String,
    pub explanation: String,
}

// ── System prompts ───────────────────────────────────────────────────────────

const SYSTEM_INCIDENT_SUMMARY: &str = "You are an SRE assistant. Produce concise, action-oriented incident summaries. No emojis, no markdown headers, no preamble. Plain prose only.";

const SYSTEM_LOG_TRIAGE: &str = "You are a log analysis assistant. Identify the most severe issues in the provided logs. Always reply with valid JSON only — no prose around it. Use one of [info, warn, error, critical] for severity.";

const SYSTEM_ALERT_EXPLAIN: &str = "You are an on-call engineer. Translate raw alert metadata into a clear, jargon-light explanation. Be specific about what to check first. No markdown.";

const SYSTEM_REMEDIATION: &str = "You are a senior SRE proposing remediation. Be conservative: prefer read-only diagnostic steps before mutating actions, and mark anything mutating or production-affecting as requires_approval=true. Reply with valid JSON only.";

const SYSTEM_ANOMALY: &str = "You analyse time-series samples and judge whether a metric is anomalous. Account for seasonality and trend. Reply with valid JSON only.";

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Cap log payloads so we never blow the context window even with very chatty
/// services. Keeps the most recent lines.
fn truncate_lines(lines: &[String], limit: usize) -> String {
    let start = lines.len().saturating_sub(limit);
    lines[start..].join("\n")
}

/// Models sometimes wrap JSON in ```json ... ``` fences. Strip them if present.
fn extract_json_block(s: &str) -> Option<&str> {
    let trimmed = s.trim();
    if let Some(rest) = trimmed.strip_prefix("```json") {
        return rest.trim_start().strip_suffix("```").map(|x| x.trim());
    }
    if let Some(rest) = trimmed.strip_prefix("```") {
        return rest.trim_start().strip_suffix("```").map(|x| x.trim());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alias::AliasRegistry;
    use crate::provider::{MockProvider, ProviderRegistry};
    use crate::router::{GatewayRouter, RoutingStrategy};

    fn engine() -> InsightsEngine {
        let providers = Arc::new(ProviderRegistry::new());
        providers.register(Arc::new(MockProvider::new("mock")));
        let aliases = Arc::new(AliasRegistry::new());
        // Route the insights test model id to the mock provider explicitly
        // so we don't depend on provider-name inference.
        aliases.register(crate::alias::ModelAlias {
            alias: "mock-model".into(),
            provider: "mock".into(),
            model: "mock-model".into(),
            description: None,
        });
        let router = Arc::new(GatewayRouter::new(
            providers,
            aliases,
            RoutingStrategy::Fixed { provider: "mock".into() },
        ));
        InsightsEngine::new(router, "mock-model")
    }

    #[tokio::test]
    async fn summarize_incident_returns_text() {
        let e = engine();
        let ctx = IncidentContext {
            title: "API 500 spike",
            severity: "P2",
            status: "Open",
            description: "Error rate jumped to 8%",
            timeline: vec![TimelineEntry {
                timestamp: "12:00",
                event_type: "alert_fired",
                message: "high_error_rate",
            }],
        };
        let s = e.summarize_incident(&ctx).await.unwrap();
        assert!(!s.is_empty());
    }

    #[test]
    fn extracts_fenced_json() {
        let raw = "```json\n{\"a\":1}\n```";
        let inner = extract_json_block(raw).unwrap();
        assert_eq!(inner, "{\"a\":1}");
    }

    #[test]
    fn truncate_keeps_tail() {
        let lines: Vec<String> = (0..10).map(|i| format!("line-{i}")).collect();
        let out = truncate_lines(&lines, 3);
        assert!(out.contains("line-9"));
        assert!(!out.contains("line-0"));
    }
}

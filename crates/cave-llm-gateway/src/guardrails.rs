//! Pre-request guardrails: PII filtering, content policy, budget and rate limits.

use crate::models::{Guardrail, GuardrailAction, GuardrailType, LlmRequest};
use crate::GatewayState;
use tracing::{info, warn};

#[derive(Debug)]
pub enum GuardrailResult {
    Pass,
    Warn(String),
    Block(String),
    /// Request was modified (e.g. PII redacted). Carry the clean version forward.
    Redact(LlmRequest),
}

/// Run all enabled guardrails against `request` in order.
/// Returns the first `Block`, accumulates `Redact` modifications, collects `Warn`s.
pub fn evaluate_guardrails(state: &GatewayState, request: &LlmRequest) -> GuardrailResult {
    let enabled: Vec<Guardrail> = {
        let g = state.guardrails.lock().unwrap();
        g.iter().filter(|g| g.enabled).cloned().collect()
    };

    let mut current = request.clone();

    for guardrail in &enabled {
        let result = match guardrail.guardrail_type {
            GuardrailType::PiiFilter => pii_filter(&current, guardrail),
            GuardrailType::ContentPolicy => content_policy(&current, guardrail),
            GuardrailType::TokenLimit => token_limit_check(&current, guardrail),
            GuardrailType::CostLimit => cost_limit_check(&current, guardrail),
            GuardrailType::RateLimit => rate_limit_check(&current, state, guardrail),
        };

        match result {
            GuardrailResult::Block(reason) => {
                warn!(guardrail = %guardrail.name, %reason, "Request blocked");
                return GuardrailResult::Block(reason);
            }
            GuardrailResult::Redact(modified) => {
                current = modified;
            }
            GuardrailResult::Warn(msg) => {
                warn!(guardrail = %guardrail.name, %msg, "Guardrail warning");
            }
            GuardrailResult::Pass => {}
        }
    }

    if current != *request {
        GuardrailResult::Redact(current)
    } else {
        GuardrailResult::Pass
    }
}

/// Detect and optionally redact PII in prompt messages.
/// Integrates conceptually with cave-pii; inline heuristics for now.
pub fn pii_filter(request: &LlmRequest, guardrail: &Guardrail) -> GuardrailResult {
    let mut modified_messages = request.messages.clone();
    let mut found_pii = false;

    for msg in &mut modified_messages {
        // Email heuristic: word containing '@' followed by a '.'
        let words: Vec<&str> = msg.content.split_whitespace().collect();
        let redacted: Vec<&str> = words
            .iter()
            .map(|w| {
                if w.contains('@') && w.contains('.') {
                    found_pii = true;
                    "[EMAIL]"
                } else {
                    w
                }
            })
            .collect();

        if found_pii {
            msg.content = redacted.join(" ");
        }

        // SSN heuristic: NNN-NN-NNNN
        if msg.content.len() >= 11 {
            let bytes = msg.content.as_bytes();
            let mut i = 0;
            while i + 10 < bytes.len() {
                if bytes[i..i + 3].iter().all(|b| b.is_ascii_digit())
                    && bytes[i + 3] == b'-'
                    && bytes[i + 4..i + 6].iter().all(|b| b.is_ascii_digit())
                    && bytes[i + 6] == b'-'
                    && bytes[i + 7..i + 11].iter().all(|b| b.is_ascii_digit())
                {
                    msg.content = msg.content[..i].to_string()
                        + "[SSN]"
                        + &msg.content[i + 11..];
                    found_pii = true;
                    break;
                }
                i += 1;
            }
        }
    }

    if !found_pii {
        return GuardrailResult::Pass;
    }

    match guardrail.action {
        GuardrailAction::Block => {
            GuardrailResult::Block("PII detected in request".to_string())
        }
        GuardrailAction::Warn => {
            GuardrailResult::Warn("PII detected in request".to_string())
        }
        GuardrailAction::Redact => {
            let mut modified = request.clone();
            modified.messages = modified_messages;
            GuardrailResult::Redact(modified)
        }
    }
}

/// Block requests containing policy-violating terms.
pub fn content_policy(request: &LlmRequest, guardrail: &Guardrail) -> GuardrailResult {
    let default_blocked = ["<script>", "DROP TABLE", "rm -rf /"];
    let blocked: Vec<String> = guardrail.config["blocked_terms"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_else(|| default_blocked.iter().map(|s| s.to_string()).collect());

    for msg in &request.messages {
        for term in &blocked {
            if msg.content.contains(term.as_str()) {
                return GuardrailResult::Block(
                    "Content policy violation: blocked term detected".to_string(),
                );
            }
        }
    }

    GuardrailResult::Pass
}

/// Reject requests whose estimated input token count exceeds the configured limit.
pub fn token_limit_check(request: &LlmRequest, guardrail: &Guardrail) -> GuardrailResult {
    let max_tokens = guardrail.config["max_input_tokens"].as_u64().unwrap_or(32_000);

    // ~4 characters per token is a common approximation.
    let estimated: u64 = request
        .messages
        .iter()
        .map(|m| m.content.len() as u64 / 4)
        .sum();

    if estimated > max_tokens {
        return GuardrailResult::Block(format!(
            "Estimated input tokens ({estimated}) exceeds limit ({max_tokens})"
        ));
    }

    GuardrailResult::Pass
}

/// Reject requests whose estimated cost exceeds the per-request threshold.
pub fn cost_limit_check(request: &LlmRequest, guardrail: &Guardrail) -> GuardrailResult {
    let max_cost_usd = guardrail.config["max_cost_usd"].as_f64().unwrap_or(1.0);

    let input_tokens: f64 = request
        .messages
        .iter()
        .map(|m| m.content.len() as f64 / 4.0)
        .sum();
    let output_tokens = request.max_tokens.unwrap_or(1024) as f64;
    // Conservative estimate: $0.01 / 1k tokens
    let estimated_cost = (input_tokens + output_tokens) / 1_000.0 * 0.01;

    if estimated_cost > max_cost_usd {
        return GuardrailResult::Block(format!(
            "Estimated cost (${estimated_cost:.4}) exceeds limit (${max_cost_usd:.2})"
        ));
    }

    GuardrailResult::Pass
}

/// Per-user/team rate limiting check.
///
/// Full sliding-window counters require persistent state (Redis / in-memory map with
/// timestamps). This stub validates the configuration path and logs intent.
/// Wire in cave-db or a tokio::sync::Mutex<HashMap<String, VecDeque<Instant>>> for
/// the real implementation.
pub fn rate_limit_check(
    request: &LlmRequest,
    _state: &GatewayState,
    guardrail: &Guardrail,
) -> GuardrailResult {
    let max_rpm = guardrail.config["max_requests_per_minute"].as_u64().unwrap_or(60);
    let scope = request
        .metadata
        .as_ref()
        .and_then(|m| m.user_id.as_deref())
        .unwrap_or("anonymous");

    info!(scope, max_rpm, "Rate limit check (sliding-window not yet implemented)");

    GuardrailResult::Pass
}

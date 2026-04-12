//! Guardrails — input/output content filtering.

use crate::error::{GatewayError, GatewayResult};
use crate::openai::{ChatCompletionRequest, ChatCompletionResponse, MessageContent};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuardrailRule {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub rule_type: RuleType,
    pub action: GuardrailAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleType {
    /// Block if input contains any of these substrings (case-insensitive)
    BlockedKeywords { keywords: Vec<String> },
    /// Block if input matches a regex pattern
    RegexBlock { pattern: String },
    /// Block if prompt length exceeds max_chars
    MaxPromptLength { max_chars: usize },
    /// Block if response contains any of these substrings
    OutputFilter { keywords: Vec<String> },
    /// Require output to not be empty
    NonEmptyOutput,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GuardrailAction {
    Block,
    Warn,
    Redact,
}

pub struct GuardrailEngine {
    rules: DashMap<String, GuardrailRule>,
}

impl GuardrailEngine {
    pub fn new() -> Self {
        let e = Self { rules: DashMap::new() };
        e.load_defaults();
        e
    }

    fn load_defaults(&self) {
        self.add_rule(GuardrailRule {
            id: "max-prompt-length".into(),
            name: "Max prompt length 32k chars".into(),
            enabled: true,
            rule_type: RuleType::MaxPromptLength { max_chars: 32_768 },
            action: GuardrailAction::Block,
        });
        self.add_rule(GuardrailRule {
            id: "non-empty-output".into(),
            name: "Response must not be empty".into(),
            enabled: true,
            rule_type: RuleType::NonEmptyOutput,
            action: GuardrailAction::Warn,
        });
    }

    pub fn add_rule(&self, rule: GuardrailRule) {
        self.rules.insert(rule.id.clone(), rule);
    }

    pub fn remove_rule(&self, id: &str) -> bool {
        self.rules.remove(id).is_some()
    }

    pub fn list_rules(&self) -> Vec<GuardrailRule> {
        self.rules.iter().map(|e| e.value().clone()).collect()
    }

    /// Check the input request. Returns Err if a blocking rule fires.
    pub fn check_input(&self, req: &ChatCompletionRequest) -> GatewayResult<Vec<String>> {
        let mut warnings = Vec::new();

        // Gather all message text
        let full_text: String = req.messages.iter()
            .filter_map(|m| m.content.as_text())
            .collect::<Vec<_>>()
            .join("\n");

        for entry in self.rules.iter() {
            let rule = entry.value();
            if !rule.enabled {
                continue;
            }

            let fired = match &rule.rule_type {
                RuleType::MaxPromptLength { max_chars } => full_text.len() > *max_chars,
                RuleType::BlockedKeywords { keywords } => {
                    let lower = full_text.to_lowercase();
                    keywords.iter().any(|kw| lower.contains(kw.as_str()))
                }
                RuleType::RegexBlock { .. } => {
                    // Regex support would need the `regex` crate; skip for now
                    false
                }
                // Output rules checked separately
                RuleType::OutputFilter { .. } | RuleType::NonEmptyOutput => false,
            };

            if fired {
                match rule.action {
                    GuardrailAction::Block => {
                        return Err(GatewayError::GuardrailBlocked {
                            rule: rule.name.clone(),
                            reason: "input violated guardrail".into(),
                        });
                    }
                    GuardrailAction::Warn | GuardrailAction::Redact => {
                        warnings.push(format!("guardrail warning: {}", rule.name));
                    }
                }
            }
        }

        Ok(warnings)
    }

    /// Check the output response. Returns Err if a blocking rule fires.
    pub fn check_output(&self, resp: &ChatCompletionResponse) -> GatewayResult<Vec<String>> {
        let mut warnings = Vec::new();

        let output_text: String = resp.choices.iter()
            .filter_map(|c| c.message.as_ref())
            .filter_map(|m| m.content.as_text())
            .collect::<Vec<_>>()
            .join("\n");

        for entry in self.rules.iter() {
            let rule = entry.value();
            if !rule.enabled {
                continue;
            }

            let fired = match &rule.rule_type {
                RuleType::OutputFilter { keywords } => {
                    let lower = output_text.to_lowercase();
                    keywords.iter().any(|kw| lower.contains(kw.as_str()))
                }
                RuleType::NonEmptyOutput => output_text.trim().is_empty(),
                _ => false,
            };

            if fired {
                match rule.action {
                    GuardrailAction::Block => {
                        return Err(GatewayError::GuardrailBlocked {
                            rule: rule.name.clone(),
                            reason: "output violated guardrail".into(),
                        });
                    }
                    GuardrailAction::Warn | GuardrailAction::Redact => {
                        warnings.push(format!("guardrail warning: {}", rule.name));
                    }
                }
            }
        }

        Ok(warnings)
    }
}

impl Default for GuardrailEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openai::{ChatMessage, Usage};

    fn make_req(text: &str) -> ChatCompletionRequest {
        ChatCompletionRequest {
            model: "gpt-4o".into(),
            messages: vec![ChatMessage::user(text)],
            temperature: None, top_p: None, max_tokens: None, stream: None,
            stop: None, presence_penalty: None, frequency_penalty: None,
            n: None, user: None, tools: None, tool_choice: None,
            response_format: None, seed: None, logprobs: None,
        }
    }

    #[test]
    fn normal_request_passes() {
        let engine = GuardrailEngine::new();
        assert!(engine.check_input(&make_req("Hello, world!")).is_ok());
    }

    #[test]
    fn max_length_blocks() {
        let engine = GuardrailEngine::new();
        let long_text = "a".repeat(40_000);
        let result = engine.check_input(&make_req(&long_text));
        assert!(result.is_err());
    }

    #[test]
    fn keyword_block_fires() {
        let engine = GuardrailEngine::new();
        engine.add_rule(GuardrailRule {
            id: "no-badword".into(),
            name: "Block badword".into(),
            enabled: true,
            rule_type: RuleType::BlockedKeywords { keywords: vec!["badword".into()] },
            action: GuardrailAction::Block,
        });
        let result = engine.check_input(&make_req("this contains badword here"));
        assert!(result.is_err());
    }

    #[test]
    fn output_filter_fires() {
        let engine = GuardrailEngine::new();
        engine.add_rule(GuardrailRule {
            id: "no-secret".into(),
            name: "No secrets in output".into(),
            enabled: true,
            rule_type: RuleType::OutputFilter { keywords: vec!["secret".into()] },
            action: GuardrailAction::Block,
        });
        let resp = ChatCompletionResponse::simple("gpt-4o", "Here is the secret key".into(), Usage::new(5, 5));
        let result = engine.check_output(&resp);
        assert!(result.is_err());
    }

    #[test]
    fn disabled_rule_not_checked() {
        let engine = GuardrailEngine::new();
        engine.add_rule(GuardrailRule {
            id: "disabled-kw".into(),
            name: "Disabled".into(),
            enabled: false,
            rule_type: RuleType::BlockedKeywords { keywords: vec!["anything".into()] },
            action: GuardrailAction::Block,
        });
        assert!(engine.check_input(&make_req("this contains anything")).is_ok());
    }
}

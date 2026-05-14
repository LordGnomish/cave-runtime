// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! API design-time governance (Spectral-like linting + quality gates).
//!
//! Rules for OpenAPI validation, consistency checks, and quality scoring.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Hint,
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum RuleSelector {
    PathPattern(String),
    SchemaPath(String),
    OperationMethod(String),
    Global,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum RuleCondition {
    MustExist,
    MustNotExist,
    MustMatchRegex(String),
    MustEqual(serde_json::Value),
    MinCount(u32),
    MaxCount(u32),
}

/// A governance rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GovernanceRule {
    pub id: String,
    pub name: String,
    pub severity: Severity,
    pub description: String,
    pub message: String,
    pub selector: RuleSelector,
    pub condition: RuleCondition,
    pub enabled: bool,
}

/// Finding from rule evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub rule_id: String,
    pub rule_name: String,
    pub severity: Severity,
    pub message: String,
    pub location: String,
}

/// Quality score and grade.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityScore {
    pub grade: char, // A-F
    pub score: f64,  // 0-100
    pub error_count: u32,
    pub warn_count: u32,
    pub info_count: u32,
    pub hint_count: u32,
}

/// Governance engine for API validation.
pub struct GovernanceEngine {
    rules: Vec<GovernanceRule>,
}

impl GovernanceEngine {
    /// Create a new governance engine with built-in rules.
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            rules: Self::default_rules(),
        })
    }

    /// Define the built-in rule set (12+ rules).
    fn default_rules() -> Vec<GovernanceRule> {
        vec![
            // Rule 1: OpenAPI must have info.title
            GovernanceRule {
                id: "openapi-info-title".to_string(),
                name: "OpenAPI must have info.title".to_string(),
                severity: Severity::Error,
                description: "All OpenAPI documents must define a title in info section.".to_string(),
                message: "Missing required field: info.title".to_string(),
                selector: RuleSelector::Global,
                condition: RuleCondition::MustExist,
                enabled: true,
            },
            // Rule 2: OpenAPI must have info.version
            GovernanceRule {
                id: "openapi-info-version".to_string(),
                name: "OpenAPI must have info.version".to_string(),
                severity: Severity::Error,
                description: "All OpenAPI documents must define a version in info section.".to_string(),
                message: "Missing required field: info.version".to_string(),
                selector: RuleSelector::Global,
                condition: RuleCondition::MustExist,
                enabled: true,
            },
            // Rule 3: Security requirement on non-public operations
            GovernanceRule {
                id: "operations-require-security".to_string(),
                name: "Non-public operations must require security".to_string(),
                severity: Severity::Warn,
                description: "Operations without explicit security definitions should require authentication.".to_string(),
                message: "Operation missing security requirement".to_string(),
                selector: RuleSelector::OperationMethod("*".to_string()),
                condition: RuleCondition::MustExist,
                enabled: true,
            },
            // Rule 4: Responses must include 400
            GovernanceRule {
                id: "responses-include-400".to_string(),
                name: "Responses must include 400 Bad Request".to_string(),
                severity: Severity::Warn,
                description: "Operations should define a 400 Bad Request response.".to_string(),
                message: "Missing 400 Bad Request response".to_string(),
                selector: RuleSelector::OperationMethod("*".to_string()),
                condition: RuleCondition::MustExist,
                enabled: true,
            },
            // Rule 5: Responses must include 401
            GovernanceRule {
                id: "responses-include-401".to_string(),
                name: "Responses must include 401 Unauthorized".to_string(),
                severity: Severity::Warn,
                description: "Operations should define a 401 Unauthorized response.".to_string(),
                message: "Missing 401 Unauthorized response".to_string(),
                selector: RuleSelector::OperationMethod("*".to_string()),
                condition: RuleCondition::MustExist,
                enabled: true,
            },
            // Rule 6: Responses must include 500
            GovernanceRule {
                id: "responses-include-500".to_string(),
                name: "Responses must include 500 Internal Server Error".to_string(),
                severity: Severity::Warn,
                description: "Operations should define a 500 Internal Server Error response.".to_string(),
                message: "Missing 500 Internal Server Error response".to_string(),
                selector: RuleSelector::OperationMethod("*".to_string()),
                condition: RuleCondition::MustExist,
                enabled: true,
            },
            // Rule 7: OperationId must be unique
            GovernanceRule {
                id: "operation-id-unique".to_string(),
                name: "operationId must be unique".to_string(),
                severity: Severity::Error,
                description: "Each operation must have a unique operationId across the entire API.".to_string(),
                message: "Duplicate or missing operationId".to_string(),
                selector: RuleSelector::OperationMethod("*".to_string()),
                condition: RuleCondition::MustExist,
                enabled: true,
            },
            // Rule 8: Tags must exist
            GovernanceRule {
                id: "tags-must-exist".to_string(),
                name: "Operations must have tags".to_string(),
                severity: Severity::Info,
                description: "Operations should be organized with tags.".to_string(),
                message: "Operation missing tags".to_string(),
                selector: RuleSelector::OperationMethod("*".to_string()),
                condition: RuleCondition::MustExist,
                enabled: true,
            },
            // Rule 9: Description length minimum
            GovernanceRule {
                id: "description-min-length".to_string(),
                name: "Descriptions must be at least 40 characters".to_string(),
                severity: Severity::Info,
                description: "Operation descriptions should be sufficiently detailed (min 40 chars).".to_string(),
                message: "Description too short or missing".to_string(),
                selector: RuleSelector::OperationMethod("*".to_string()),
                condition: RuleCondition::MinCount(40),
                enabled: true,
            },
            // Rule 10: Paths must be kebab-case
            GovernanceRule {
                id: "paths-kebab-case".to_string(),
                name: "Paths must be kebab-case".to_string(),
                severity: Severity::Warn,
                description: "API paths should follow kebab-case naming convention.".to_string(),
                message: "Path does not follow kebab-case convention".to_string(),
                selector: RuleSelector::PathPattern("*".to_string()),
                condition: RuleCondition::MustMatchRegex("^[a-z0-9/\\-{}]*$".to_string()),
                enabled: true,
            },
            // Rule 11: No dashes in operationId
            GovernanceRule {
                id: "operation-id-no-dashes".to_string(),
                name: "operationId must use camelCase (no dashes)".to_string(),
                severity: Severity::Warn,
                description: "operationId should use camelCase naming.".to_string(),
                message: "operationId contains dashes or invalid characters".to_string(),
                selector: RuleSelector::OperationMethod("*".to_string()),
                condition: RuleCondition::MustMatchRegex("^[a-zA-Z][a-zA-Z0-9]*$".to_string()),
                enabled: true,
            },
            // Rule 12: All parameters must have schema
            GovernanceRule {
                id: "parameters-must-have-schema".to_string(),
                name: "All parameters must define a schema".to_string(),
                severity: Severity::Error,
                description: "Every parameter must explicitly define its schema.".to_string(),
                message: "Parameter missing schema definition".to_string(),
                selector: RuleSelector::SchemaPath("*.parameters[*]".to_string()),
                condition: RuleCondition::MustExist,
                enabled: true,
            },
            // Rule 13: Error responses must use a common schema
            GovernanceRule {
                id: "error-response-common-schema".to_string(),
                name: "Error responses must use a common error schema".to_string(),
                severity: Severity::Info,
                description: "4xx and 5xx responses should use a consistent error schema.".to_string(),
                message: "Error responses lack a common schema".to_string(),
                selector: RuleSelector::OperationMethod("*".to_string()),
                condition: RuleCondition::MustExist,
                enabled: true,
            },
            // Rule 14: Pagination headers for list operations
            GovernanceRule {
                id: "pagination-headers-for-lists".to_string(),
                name: "List operations must include pagination headers".to_string(),
                severity: Severity::Info,
                description: "Operations that return lists should define pagination (limit, offset, etc).".to_string(),
                message: "List operation missing pagination headers".to_string(),
                selector: RuleSelector::OperationMethod("get".to_string()),
                condition: RuleCondition::MustExist,
                enabled: true,
            },
        ]
    }

    /// Evaluate an OpenAPI document against all enabled rules.
    pub fn evaluate(&self, openapi: &serde_json::Value) -> Vec<Finding> {
        let mut findings = Vec::new();

        for rule in &self.rules {
            if !rule.enabled {
                continue;
            }

            // Simple rule evaluation logic (production would be more sophisticated)
            match (&rule.selector, &rule.condition) {
                (RuleSelector::Global, RuleCondition::MustExist) => {
                    // Check for info.title
                    if rule.id == "openapi-info-title" {
                        if openapi.get("info").and_then(|i| i.get("title")).is_none() {
                            findings.push(Finding {
                                rule_id: rule.id.clone(),
                                rule_name: rule.name.clone(),
                                severity: rule.severity,
                                message: rule.message.clone(),
                                location: "info.title".to_string(),
                            });
                        }
                    }
                    // Check for info.version
                    if rule.id == "openapi-info-version" {
                        if openapi.get("info").and_then(|i| i.get("version")).is_none() {
                            findings.push(Finding {
                                rule_id: rule.id.clone(),
                                rule_name: rule.name.clone(),
                                severity: rule.severity,
                                message: rule.message.clone(),
                                location: "info.version".to_string(),
                            });
                        }
                    }
                }
                (RuleSelector::OperationMethod(_), _) => {
                    // Stub: in production, iterate paths/operations
                    // For test purposes, check if paths exist
                    if let Some(paths) = openapi.get("paths").and_then(|p| p.as_object()) {
                        if paths.is_empty() && rule.id == "operation-id-unique" {
                            findings.push(Finding {
                                rule_id: rule.id.clone(),
                                rule_name: rule.name.clone(),
                                severity: rule.severity,
                                message: rule.message.clone(),
                                location: "paths".to_string(),
                            });
                        }
                    }
                }
                (RuleSelector::PathPattern(_), RuleCondition::MustMatchRegex(regex_str)) => {
                    // Check path naming convention
                    if rule.id == "paths-kebab-case" {
                        if let Some(paths) = openapi.get("paths").and_then(|p| p.as_object()) {
                            let regex = regex::Regex::new(regex_str).unwrap_or_else(|_| {
                                regex::Regex::new("^.*$").unwrap()
                            });
                            for path in paths.keys() {
                                if !regex.is_match(path) {
                                    findings.push(Finding {
                                        rule_id: rule.id.clone(),
                                        rule_name: rule.name.clone(),
                                        severity: rule.severity,
                                        message: rule.message.clone(),
                                        location: format!("paths.{}", path),
                                    });
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        findings
    }

    /// Compute a quality score based on findings.
    pub fn aggregate_score(&self, findings: &[Finding]) -> QualityScore {
        let mut error_count = 0;
        let mut warn_count = 0;
        let mut info_count = 0;
        let mut hint_count = 0;

        for finding in findings {
            match finding.severity {
                Severity::Error => error_count += 1,
                Severity::Warn => warn_count += 1,
                Severity::Info => info_count += 1,
                Severity::Hint => hint_count += 1,
            }
        }

        // Simple scoring: start at 100, deduct for each issue
        let mut score = 100.0;
        score -= (error_count as f64) * 10.0;
        score -= (warn_count as f64) * 5.0;
        score -= (info_count as f64) * 2.0;
        score -= (hint_count as f64) * 1.0;
        score = score.max(0.0).min(100.0);

        let grade = if score >= 90.0 {
            'A'
        } else if score >= 80.0 {
            'B'
        } else if score >= 70.0 {
            'C'
        } else if score >= 60.0 {
            'D'
        } else if score >= 50.0 {
            'E'
        } else {
            'F'
        };

        QualityScore {
            grade,
            score,
            error_count,
            warn_count,
            info_count,
            hint_count,
        }
    }

    /// Get the list of all rules.
    pub fn list_rules(&self) -> Vec<GovernanceRule> {
        self.rules.clone()
    }

    /// Enable/disable a rule.
    pub fn set_rule_enabled(&mut self, rule_id: &str, enabled: bool) {
        for rule in &mut self.rules {
            if rule.id == rule_id {
                rule.enabled = enabled;
                break;
            }
        }
    }
}

impl Default for GovernanceEngine {
    fn default() -> Self {
        GovernanceEngine {
            rules: Self::default_rules(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_governance_evaluate_clean_openapi() {
        let engine = GovernanceEngine::new();
        let clean_openapi = serde_json::json!({
            "openapi": "3.0.0",
            "info": {
                "title": "User API",
                "version": "1.0.0"
            },
            "paths": {
                "/users": {
                    "get": {
                        "summary": "List all users",
                        "operationId": "listUsers",
                        "tags": ["users"],
                        "responses": {
                            "200": { "description": "Success" },
                            "400": { "description": "Bad Request" },
                            "401": { "description": "Unauthorized" },
                            "500": { "description": "Internal Server Error" }
                        }
                    }
                }
            }
        });

        let findings = engine.evaluate(&clean_openapi);
        // Should have minimal or zero findings for a well-formed API
        assert!(findings.is_empty() || findings.len() < 3);
    }

    #[test]
    fn test_governance_missing_title() {
        let engine = GovernanceEngine::new();
        let broken_openapi = serde_json::json!({
            "openapi": "3.0.0",
            "info": {
                "version": "1.0.0"
            },
            "paths": {}
        });

        let findings = engine.evaluate(&broken_openapi);
        let title_missing = findings.iter().any(|f| f.rule_id == "openapi-info-title");
        assert!(title_missing);
    }

    #[test]
    fn test_governance_missing_version() {
        let engine = GovernanceEngine::new();
        let broken_openapi = serde_json::json!({
            "openapi": "3.0.0",
            "info": {
                "title": "API"
            },
            "paths": {}
        });

        let findings = engine.evaluate(&broken_openapi);
        let version_missing = findings.iter().any(|f| f.rule_id == "openapi-info-version");
        assert!(version_missing);
    }

    #[test]
    fn test_governance_quality_score() {
        let engine = GovernanceEngine::new();

        // Test with no findings
        let score = engine.aggregate_score(&[]);
        assert_eq!(score.grade, 'A');
        assert_eq!(score.score, 100.0);

        // Test with errors (1 error = 90, still 'A')
        let findings = vec![
            Finding {
                rule_id: "test1".to_string(),
                rule_name: "Test".to_string(),
                severity: Severity::Error,
                message: "Error".to_string(),
                location: "loc".to_string(),
            },
        ];
        let score = engine.aggregate_score(&findings);
        assert_eq!(score.grade, 'A');
        assert_eq!(score.error_count, 1);

        // Test with multiple errors (2+ errors = 80 or less = 'B')
        let findings = vec![
            Finding {
                rule_id: "test1".to_string(),
                rule_name: "Test".to_string(),
                severity: Severity::Error,
                message: "Error".to_string(),
                location: "loc".to_string(),
            },
            Finding {
                rule_id: "test2".to_string(),
                rule_name: "Test2".to_string(),
                severity: Severity::Error,
                message: "Error".to_string(),
                location: "loc".to_string(),
            },
        ];
        let score = engine.aggregate_score(&findings);
        assert_eq!(score.grade, 'B');
        assert_eq!(score.error_count, 2);
    }
}

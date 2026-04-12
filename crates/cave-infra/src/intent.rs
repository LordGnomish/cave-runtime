//! Intent parsing — from natural language to structured `ParsedIntent`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::providers::ResourceType;

// ── InfraIntent ───────────────────────────────────────────────────────────────

/// A user's natural-language infrastructure request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InfraIntent {
    pub id: Uuid,
    /// Raw, free-form description from the user.
    pub description: String,
    pub tenant_id: String,
    pub submitted_by: Uuid,
    pub submitted_at: DateTime<Utc>,
    /// Populated after the intent is parsed.
    pub parsed: Option<ParsedIntent>,
}

impl InfraIntent {
    pub fn new(description: &str, tenant_id: &str, submitted_by: Uuid) -> Self {
        Self {
            id: Uuid::new_v4(),
            description: description.to_string(),
            tenant_id: tenant_id.to_string(),
            submitted_by,
            submitted_at: Utc::now(),
            parsed: None,
        }
    }

    pub fn with_parsed(mut self, parsed: ParsedIntent) -> Self {
        self.parsed = Some(parsed);
        self
    }
}

// ── ParsedIntent ──────────────────────────────────────────────────────────────

/// Structured representation of what the user asked for.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedIntent {
    pub resources: Vec<ResourceRequest>,
    /// Provider names inferred from the description (e.g. "hetzner", "azure").
    pub provider_hints: Vec<String>,
    pub environment: String,
    /// Cross-cutting constraints (e.g. "high-availability", "cost-optimized").
    pub constraints: Vec<String>,
}

// ── ResourceRequest ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceRequest {
    pub resource_type: ResourceType,
    pub name: String,
    pub count: u32,
    /// Type-specific spec — shape depends on `resource_type`.
    pub spec: serde_json::Value,
}

// ── IntentParser ──────────────────────────────────────────────────────────────

/// Keyword-based intent parser.
///
/// A production implementation would call an LLM here; this version
/// uses simple string matching so the crate compiles and tests pass
/// without external dependencies.
pub struct IntentParser;

impl IntentParser {
    pub fn new() -> Self {
        Self
    }

    /// Parse a raw `InfraIntent` into a `ParsedIntent`.
    pub fn parse(&self, intent: &InfraIntent) -> ParsedIntent {
        let desc = intent.description.to_lowercase();
        let mut resources = Vec::new();

        // Detect compute resources.
        if desc.contains("vm")
            || desc.contains("server")
            || desc.contains("machine")
            || desc.contains("node")
            || desc.contains("instance")
        {
            let count = Self::count_from_description(&desc).max(1);
            resources.push(ResourceRequest {
                resource_type: ResourceType::Vm,
                name: "vm".to_string(),
                count,
                spec: serde_json::json!({ "inferred": true }),
            });
        }

        // Detect networking resources.
        if desc.contains("vpc")
            || desc.contains("network")
            || desc.contains("subnet")
        {
            resources.push(ResourceRequest {
                resource_type: ResourceType::Vpc,
                name: "vpc".to_string(),
                count: 1,
                spec: serde_json::json!({ "inferred": true }),
            });
        }

        // Detect block / object storage.
        if desc.contains("storage")
            || desc.contains("disk")
            || desc.contains("bucket")
        {
            resources.push(ResourceRequest {
                resource_type: ResourceType::BlockStorage,
                name: "storage".to_string(),
                count: 1,
                spec: serde_json::json!({ "inferred": true }),
            });
        }

        // Detect DNS resources.
        if desc.contains("dns") || desc.contains("domain") {
            resources.push(ResourceRequest {
                resource_type: ResourceType::DnsRecord,
                name: "dns".to_string(),
                count: 1,
                spec: serde_json::json!({ "inferred": true }),
            });
        }

        // Provider hints.
        let all_providers = [
            "hetzner", "azure", "aws", "gcp", "digitalocean", "cloudflare",
            "linode", "vultr", "ovh",
        ];
        let provider_hints: Vec<String> = all_providers
            .iter()
            .filter(|p| desc.contains(**p))
            .map(|p| p.to_string())
            .collect();

        // Environment detection.
        let environment = if desc.contains("prod") || desc.contains("production") {
            "production"
        } else if desc.contains("staging") {
            "staging"
        } else if desc.contains("dev") || desc.contains("development") {
            "development"
        } else {
            "unknown"
        }
        .to_string();

        // Constraint extraction.
        let mut constraints = Vec::new();
        if desc.contains("high-availability") || desc.contains("ha ") || desc.contains(" ha") {
            constraints.push("high-availability".to_string());
        }
        if desc.contains("cost") || desc.contains("cheap") || desc.contains("budget") {
            constraints.push("cost-optimized".to_string());
        }

        ParsedIntent {
            resources,
            provider_hints,
            environment,
            constraints,
        }
    }

    /// Extract the first integer found in the description, e.g. "3 vms" → 3.
    fn count_from_description(desc: &str) -> u32 {
        let mut digits = String::new();
        for ch in desc.chars() {
            if ch.is_ascii_digit() {
                digits.push(ch);
            } else if !digits.is_empty() {
                break;
            }
        }
        digits.parse().unwrap_or(1)
    }
}

impl Default for IntentParser {
    fn default() -> Self {
        Self::new()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_vm_intent() {
        let parser = IntentParser::new();
        let user = Uuid::new_v4();
        let intent = InfraIntent::new(
            "Create a 3-node k8s cluster on Hetzner with 50GB persistent storage",
            "tenant-1",
            user,
        );

        let parsed = parser.parse(&intent);

        // Should detect VMs (nodes) and storage.
        let types: Vec<&ResourceType> = parsed.resources.iter().map(|r| &r.resource_type).collect();
        assert!(types.contains(&&ResourceType::Vm), "should have VM resource");
        assert!(types.contains(&&ResourceType::BlockStorage), "should have storage resource");

        // Should recognise Hetzner as provider hint.
        assert!(parsed.provider_hints.contains(&"hetzner".to_string()));
    }

    #[test]
    fn test_parse_network_intent() {
        let parser = IntentParser::new();
        let user = Uuid::new_v4();
        let intent = InfraIntent::new(
            "Set up a VPC network with subnets in AWS for production",
            "tenant-2",
            user,
        );

        let parsed = parser.parse(&intent);

        let types: Vec<&ResourceType> = parsed.resources.iter().map(|r| &r.resource_type).collect();
        assert!(types.contains(&&ResourceType::Vpc), "should have VPC resource");
        assert!(parsed.provider_hints.contains(&"aws".to_string()));
        assert_eq!(parsed.environment, "production");
    }
}

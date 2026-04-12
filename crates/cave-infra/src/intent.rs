<<<<<<< HEAD
<<<<<<< HEAD
=======
>>>>>>> claude/interesting-khorana
//! Intent engine — parse, resolve dependencies, validate, diff state.

use crate::models::{
    InfraIntent, InfraState, McpProvider, PolicyCheck, ResourceDeclaration, StepAction,
};
use chrono::Utc;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// Parse natural language or structured YAML into an InfraIntent.
///
/// In production this is the first hop to the local LLM — the NL description is
/// sent to ollama/llama.cpp which returns structured resource declarations. Here we
/// implement deterministic heuristics so the module compiles and runs without a GPU.
pub fn parse_intent(description: &str, yaml: Option<&str>) -> Result<InfraIntent, IntentError> {
    let structured = if let Some(yml) = yaml {
        let val: serde_yaml::Value = serde_yaml::from_str(yml)
            .map_err(|e| IntentError::ParseError(e.to_string()))?;
        Some(val)
    } else {
        None
    };

    let resources = if let Some(ref s) = structured {
        parse_resources_from_yaml(s)?
    } else {
        infer_resources_from_nl(description)
    };

    Ok(InfraIntent {
        id: Uuid::new_v4(),
        description: description.to_string(),
        structured,
        resources,
        constraints: extract_constraints(description),
        created_at: Utc::now(),
    })
}

/// Topological sort of resource names by their declared dependency order.
pub fn resolve_dependencies(intent: &InfraIntent) -> Result<Vec<String>, IntentError> {
    let mut order = Vec::new();
    let mut visited = HashSet::new();
    let names: Vec<String> = intent.resources.iter().map(|r| r.name.clone()).collect();

    for name in &names {
        if !visited.contains(name) {
            topo_visit(name, &mut visited, &mut order);
        }
    }
    Ok(order)
}

fn topo_visit(node: &str, visited: &mut HashSet<String>, order: &mut Vec<String>) {
    if !visited.insert(node.to_string()) {
        return;
    }
    order.push(node.to_string());
}

/// Validate intent against registered providers; returns policy check results.
pub fn validate_intent(
    intent: &InfraIntent,
    providers: &[McpProvider],
) -> Result<Vec<PolicyCheck>, IntentError> {
    let registered: HashSet<&str> = providers.iter().map(|p| p.name.as_str()).collect();
    let mut checks = Vec::new();

    for resource in &intent.resources {
        let passed = registered.contains(resource.provider.as_str());
        checks.push(PolicyCheck {
            id: Uuid::new_v4(),
            policy_name: format!("provider-registered:{}", resource.provider),
            passed,
            violations: if passed {
                vec![]
            } else {
                vec![format!(
                    "Provider '{}' is not registered. Available: {:?}",
                    resource.provider,
                    registered.iter().copied().collect::<Vec<_>>()
                )]
            },
            evaluated_at: Utc::now(),
        });
    }
    Ok(checks)
}

/// Compare desired intent against current state — produce a list of changes needed.
pub fn diff_state(intent: &InfraIntent, current: &InfraState) -> Vec<ChangesetEntry> {
    let existing_names: HashSet<String> =
        current.resources.values().map(|r| r.name.clone()).collect();

    intent
        .resources
        .iter()
        .map(|decl| {
            let action = if existing_names.contains(&decl.name) {
                // Simplified: name match = no-op. A real diff would compare config hashes.
                StepAction::NoOp
            } else {
                StepAction::Create
            };
            ChangesetEntry {
                action,
                resource_name: decl.name.clone(),
                provider: decl.provider.clone(),
                resource_type: decl.resource_type.clone(),
            }
        })
        .collect()
}

#[derive(Debug, Clone)]
pub struct ChangesetEntry {
    pub action: StepAction,
    pub resource_name: String,
    pub provider: String,
    pub resource_type: String,
}

// ── Private helpers ─────────────────────────────────────────────────────────

fn parse_resources_from_yaml(
    yaml: &serde_yaml::Value,
) -> Result<Vec<ResourceDeclaration>, IntentError> {
    let Some(seq) = yaml.get("resources").and_then(|v| v.as_sequence()) else {
        return Ok(vec![]);
    };

    seq.iter()
        .map(|item| {
            let name = item
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| IntentError::ParseError("resource missing 'name'".to_string()))?;
            let provider = item
                .get("provider")
                .and_then(|v| v.as_str())
                .unwrap_or("aws");
            let resource_type = item
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("generic");
            Ok(ResourceDeclaration {
                name: name.to_string(),
                provider: provider.to_string(),
                resource_type: resource_type.to_string(),
                config: HashMap::new(),
            })
        })
        .collect()
}

/// Heuristic NL → resource inference. In production the LLM does this.
fn infer_resources_from_nl(description: &str) -> Vec<ResourceDeclaration> {
    let lower = description.to_lowercase();
    let provider = infer_provider(&lower);
    let mut resources = Vec::new();

    if lower.contains("postgres") || lower.contains("rds") || lower.contains("database") {
        resources.push(ResourceDeclaration {
            name: "postgres-cluster".to_string(),
            provider: provider.clone(),
            resource_type: "rds_cluster".to_string(),
            config: HashMap::new(),
        });
    }
    if lower.contains("bucket") || lower.contains("s3") || lower.contains("storage") {
        resources.push(ResourceDeclaration {
            name: "storage-bucket".to_string(),
            provider: provider.clone(),
            resource_type: "object_storage".to_string(),
            config: HashMap::new(),
        });
    }
    if lower.contains(" vm")
        || lower.contains("instance")
        || lower.contains("server")
        || lower.contains("ec2")
    {
        resources.push(ResourceDeclaration {
            name: "compute-instance".to_string(),
            provider: provider.clone(),
            resource_type: "virtual_machine".to_string(),
            config: HashMap::new(),
        });
    }
    if lower.contains("kubernetes") || lower.contains("k8s") {
        resources.push(ResourceDeclaration {
            name: "k8s-cluster".to_string(),
            provider: provider.clone(),
            resource_type: "kubernetes_cluster".to_string(),
            config: HashMap::new(),
        });
    }
    resources
}

fn infer_provider(lower: &str) -> String {
    if lower.contains("azure") || lower.contains("eastus") {
        "azure".to_string()
    } else if lower.contains("gcp") || lower.contains("google") {
        "gcp".to_string()
    } else if lower.contains("hetzner") {
        "hetzner".to_string()
    } else {
        "aws".to_string()
    }
}

fn extract_constraints(description: &str) -> Vec<String> {
    let mut constraints = Vec::new();
    let lower = description.to_lowercase();

    for region in &[
        "eu-west-1",
        "eu-west-2",
        "us-east-1",
        "us-west-2",
        "ap-southeast-1",
    ] {
        if lower.contains(region) {
            constraints.push(format!("region:{region}"));
        }
    }

    for n in 1u32..=10 {
        let variants = [format!("{n}-node"), format!("{n} node")];
        if variants.iter().any(|v| lower.contains(v.as_str())) {
            constraints.push(format!("node_count:{n}"));
        }
    }
    constraints
}

#[derive(Debug, thiserror::Error)]
pub enum IntentError {
    #[error("parse error: {0}")]
    ParseError(String),
    #[error("validation error: {0}")]
    ValidationError(String),
<<<<<<< HEAD
=======
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
>>>>>>> claude/great-sanderson
=======
>>>>>>> claude/interesting-khorana
}

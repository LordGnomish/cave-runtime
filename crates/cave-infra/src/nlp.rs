// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Natural language to infrastructure intent parser.

use crate::resource::{ResourceKind, ResourceSpec};
use std::collections::HashMap;

/// Parsed intent from a natural language infrastructure request.
#[derive(Debug, Clone)]
pub struct InfraIntent {
    pub action: IntentAction,
    pub resource_specs: Vec<ResourceSpec>,
    pub raw_text: String,
    pub confidence: f32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntentAction {
    Provision,
    Scale,
    Destroy,
    Inspect,
    Plan,
    Unknown,
}

/// Parse a natural language request into a structured infrastructure intent.
pub fn parse_intent(text: &str) -> InfraIntent {
    let lower = text.to_lowercase();
    let action = detect_action(&lower);
    let specs = extract_specs(&lower, &action);
    let confidence = if action == IntentAction::Unknown {
        0.2
    } else {
        0.8
    };

    InfraIntent {
        action,
        resource_specs: specs,
        raw_text: text.to_string(),
        confidence,
    }
}

fn detect_action(text: &str) -> IntentAction {
    if text.contains("create")
        || text.contains("provision")
        || text.contains("spin up")
        || text.contains("deploy")
        || text.contains("set up")
        || text.contains("launch")
        || text.contains("add")
        || text.contains("new ")
    {
        IntentAction::Provision
    } else if text.contains("scale")
        || text.contains("resize")
        || text.contains("upgrade")
        || text.contains("downgrade")
        || text.contains("increase")
        || text.contains("decrease")
    {
        IntentAction::Scale
    } else if text.contains("delete")
        || text.contains("destroy")
        || text.contains("remove")
        || text.contains("terminate")
        || text.contains("tear down")
    {
        IntentAction::Destroy
    } else if text.contains("show")
        || text.contains("list")
        || text.contains("describe")
        || text.contains("status")
        || text.contains("inspect")
        || text.contains("check")
    {
        IntentAction::Inspect
    } else if text.contains("plan")
        || text.contains("preview")
        || text.contains("what would")
        || text.contains("dry run")
    {
        IntentAction::Plan
    } else {
        IntentAction::Unknown
    }
}

fn extract_specs(text: &str, action: &IntentAction) -> Vec<ResourceSpec> {
    if matches!(action, IntentAction::Inspect | IntentAction::Unknown) {
        return vec![];
    }

    let mut specs = Vec::new();

    // Server patterns
    if text.contains("server")
        || text.contains("vm")
        || text.contains("instance")
        || text.contains("machine")
        || text.contains("node")
    {
        let mut props = HashMap::new();
        props.insert("os".into(), serde_json::json!("ubuntu-22.04"));

        // CPU extraction
        if let Some(cpu) = extract_number_before(text, "cpu")
            .or_else(|| extract_number_before(text, "core"))
            .or_else(|| extract_number_before(text, "vcpu"))
        {
            props.insert("cpu".into(), serde_json::json!(cpu));
        } else {
            props.insert("cpu".into(), serde_json::json!(2));
        }

        // Memory extraction
        if let Some(mem) = extract_memory_gb(text) {
            props.insert("memory_gb".into(), serde_json::json!(mem));
        } else {
            props.insert("memory_gb".into(), serde_json::json!(4));
        }

        // Disk extraction
        if let Some(disk) = extract_storage_gb(text) {
            props.insert("disk_gb".into(), serde_json::json!(disk));
        }

        let name = extract_name(text, "server").unwrap_or_else(|| "server-01".into());
        specs.push(ResourceSpec {
            kind: ResourceKind::Server,
            name,
            provider: detect_provider(text),
            properties: props,
            depends_on: vec![],
            tags: HashMap::new(),
        });
    }

    // Database patterns
    if text.contains("database")
        || text.contains("postgres")
        || text.contains("mysql")
        || text.contains("db ")
    {
        let mut props = HashMap::new();
        props.insert(
            "engine".into(),
            serde_json::json!(if text.contains("mysql") {
                "mysql"
            } else {
                "postgresql"
            }),
        );
        props.insert("version".into(), serde_json::json!("16"));
        if let Some(storage) = extract_storage_gb(text) {
            props.insert("storage_gb".into(), serde_json::json!(storage));
        } else {
            props.insert("storage_gb".into(), serde_json::json!(100));
        }

        let name = extract_name(text, "database").unwrap_or_else(|| "db-01".into());
        specs.push(ResourceSpec {
            kind: ResourceKind::Database,
            name,
            provider: detect_provider(text),
            properties: props,
            depends_on: vec![],
            tags: HashMap::new(),
        });
    }

    // Network patterns
    if text.contains("network") || text.contains("vpc") || text.contains("vnet") {
        let mut props = HashMap::new();
        props.insert("cidr".into(), serde_json::json!("10.0.0.0/16"));
        let name = extract_name(text, "network").unwrap_or_else(|| "network-01".into());
        specs.push(ResourceSpec {
            kind: ResourceKind::Network,
            name,
            provider: detect_provider(text),
            properties: props,
            depends_on: vec![],
            tags: HashMap::new(),
        });
    }

    // Load balancer patterns
    if text.contains("load balancer") || text.contains("loadbalancer") || text.contains("lb ") {
        let mut props = HashMap::new();
        props.insert("type".into(), serde_json::json!("http"));
        let name = extract_name(text, "lb").unwrap_or_else(|| "lb-01".into());
        specs.push(ResourceSpec {
            kind: ResourceKind::LoadBalancer,
            name,
            provider: detect_provider(text),
            properties: props,
            depends_on: vec![],
            tags: HashMap::new(),
        });
    }

    specs
}

fn extract_number_before(text: &str, keyword: &str) -> Option<i64> {
    let pos = text.find(keyword)?;
    let before = &text[..pos].trim_end();
    let last_word: &str = before.split_whitespace().last()?;
    last_word.parse().ok()
}

fn extract_memory_gb(text: &str) -> Option<i64> {
    // Patterns: "8GB", "8 GB", "8gb", "8g"
    let re_patterns = [" gb ", "gb ", " gb", "gib", " gib"];
    for pattern in re_patterns {
        if let Some(pos) = text.find(pattern) {
            let before = &text[..pos].trim_end();
            if let Some(last) = before.split_whitespace().last() {
                if let Ok(n) = last.parse::<i64>() {
                    return Some(n);
                }
            }
        }
    }
    None
}

fn extract_storage_gb(text: &str) -> Option<i64> {
    // Look for patterns like "100gb disk", "500 gb storage"
    for pattern in [" gb disk", " gb storage", "gb ssd", "gb hdd"] {
        if let Some(pos) = text.find(pattern) {
            let before = &text[..pos].trim_end();
            if let Some(last) = before.split_whitespace().last() {
                if let Ok(n) = last.parse::<i64>() {
                    return Some(n);
                }
            }
        }
    }
    None
}

fn extract_name(text: &str, kind: &str) -> Option<String> {
    // Look for patterns like "called foo", "named foo", "named 'foo'"
    for prefix in ["called ", "named "] {
        if let Some(pos) = text.find(prefix) {
            let after = &text[pos + prefix.len()..];
            let name: String = after
                .split_whitespace()
                .next()?
                .trim_matches(|c: char| c == '\'' || c == '"')
                .to_string();
            if !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
            {
                return Some(name);
            }
        }
    }
    None
}

fn detect_provider(text: &str) -> String {
    if text.contains("bare metal") || text.contains("baremetal") || text.contains("on-prem") {
        "bare-metal"
    } else {
        "noop"
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_provision_server() {
        let intent = parse_intent("Create a server with 8 CPU and 16 GB memory");
        assert_eq!(intent.action, IntentAction::Provision);
        assert!(!intent.resource_specs.is_empty());
        let spec = &intent.resource_specs[0];
        assert_eq!(spec.kind, ResourceKind::Server);
        assert_eq!(spec.properties["cpu"], serde_json::json!(8));
    }

    #[test]
    fn parse_provision_database() {
        let intent = parse_intent("Deploy a postgres database with 100gb storage");
        assert_eq!(intent.action, IntentAction::Provision);
        let db = intent
            .resource_specs
            .iter()
            .find(|s| s.kind == ResourceKind::Database);
        assert!(db.is_some());
    }

    #[test]
    fn parse_destroy_action() {
        let intent = parse_intent("Delete the server named web-01");
        assert_eq!(intent.action, IntentAction::Destroy);
    }

    #[test]
    fn parse_inspect_action() {
        let intent = parse_intent("Show me all running servers");
        assert_eq!(intent.action, IntentAction::Inspect);
    }

    #[test]
    fn parse_plan_action() {
        let intent = parse_intent("What would happen if I deploy a new network?");
        assert_eq!(intent.action, IntentAction::Plan);
    }

    #[test]
    fn unknown_action_has_low_confidence() {
        let intent = parse_intent("hmm what do you think about clouds");
        assert_eq!(intent.action, IntentAction::Unknown);
        assert!(intent.confidence < 0.5);
    }

    #[test]
    fn name_extraction() {
        let intent = parse_intent("Create a server called production-api");
        let spec = &intent.resource_specs[0];
        assert_eq!(spec.name, "production-api");
    }
}

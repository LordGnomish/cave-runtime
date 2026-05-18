// SPDX-License-Identifier: AGPL-3.0-or-later
//! Rule engine: evaluates Falco-style conditions against security events.

use crate::models::{Condition, Priority, SecurityAlert, SecurityEvent, SecurityRule};
use chrono::Utc;
use uuid::Uuid;

/// Recursively evaluate a condition against a security event.
pub fn evaluate_condition(condition: &Condition, event: &SecurityEvent) -> bool {
    match condition {
        Condition::ProcessName { value, exact } => event
            .process_name
            .as_deref()
            .map_or(false, |name| {
                if *exact {
                    name == value.as_str()
                } else {
                    name.contains(value.as_str())
                }
            }),
        Condition::FilePath { prefix } => event
            .file_path
            .as_deref()
            .map_or(false, |path| path.starts_with(prefix.as_str())),
        Condition::NetworkPort { port } => event.network_port.map_or(false, |p| p == *port),
        Condition::IsRoot => event.is_root,
        Condition::Syscall { name } => event
            .syscall
            .as_deref()
            .map_or(false, |s| s == name.as_str()),
        Condition::ContainerImage { prefix } => event
            .container_image
            .as_deref()
            .map_or(false, |img| img.starts_with(prefix.as_str())),
        Condition::And { conditions } => conditions.iter().all(|c| evaluate_condition(c, event)),
        Condition::Or { conditions } => conditions.iter().any(|c| evaluate_condition(c, event)),
        Condition::Not { condition } => !evaluate_condition(condition, event),
    }
}

/// Evaluate an event against all enabled rules, returning matching alerts.
pub fn evaluate_rules(rules: &[SecurityRule], event: &SecurityEvent) -> Vec<SecurityAlert> {
    rules
        .iter()
        .filter(|r| r.enabled)
        .filter(|r| evaluate_condition(&r.condition, event))
        .map(|r| SecurityAlert {
            id: Uuid::new_v4(),
            rule_id: r.id,
            rule_name: r.name.clone(),
            priority: r.priority,
            message: format!("Rule '{}' triggered: {}", r.name, r.description),
            event: event.clone(),
            timestamp: Utc::now(),
            acknowledged: false,
        })
        .collect()
}

/// Built-in rules (Falco-equivalent defaults).
pub fn builtin_rules() -> Vec<SecurityRule> {
    vec![
        SecurityRule::new(
            "shell_in_container",
            "Shell execution detected inside a container",
            Priority::Warning,
            Condition::Or {
                conditions: vec![
                    Condition::ProcessName { value: "sh".to_string(), exact: true },
                    Condition::ProcessName { value: "bash".to_string(), exact: true },
                    Condition::ProcessName { value: "zsh".to_string(), exact: true },
                ],
            },
        ),
        SecurityRule::new(
            "sensitive_file_access",
            "Access to sensitive system files detected",
            Priority::Critical,
            Condition::Or {
                conditions: vec![
                    Condition::FilePath { prefix: "/etc/shadow".to_string() },
                    Condition::FilePath { prefix: "/etc/passwd".to_string() },
                    Condition::FilePath { prefix: "/proc/sys".to_string() },
                ],
            },
        ),
        SecurityRule::new(
            "privilege_escalation",
            "Process running as root (non-init) detected",
            Priority::Alert,
            Condition::And {
                conditions: vec![
                    Condition::IsRoot,
                    Condition::Not {
                        condition: Box::new(Condition::ProcessName {
                            value: "init".to_string(),
                            exact: true,
                        }),
                    },
                ],
            },
        ),
        SecurityRule::new(
            "suspicious_network_port",
            "Connection to known suspicious port detected",
            Priority::Notice,
            Condition::Or {
                conditions: vec![
                    Condition::NetworkPort { port: 4444 },
                    Condition::NetworkPort { port: 1337 },
                    Condition::NetworkPort { port: 31337 },
                ],
            },
        ),
        SecurityRule::new(
            "ptrace_syscall",
            "ptrace syscall detected — possible process injection",
            Priority::Error,
            Condition::Syscall { name: "ptrace".to_string() },
        ),
        SecurityRule::new(
            "untrusted_registry",
            "Container image pulled from untrusted registry",
            Priority::Warning,
            // Fires only when a container image IS present AND it's not from a trusted registry.
            // (ContainerImage { prefix: "" } matches any non-None image via starts_with(""))
            Condition::And {
                conditions: vec![
                    Condition::ContainerImage { prefix: String::new() },
                    Condition::Not {
                        condition: Box::new(Condition::Or {
                            conditions: vec![
                                Condition::ContainerImage { prefix: "gcr.io/".to_string() },
                                Condition::ContainerImage { prefix: "ghcr.io/".to_string() },
                                Condition::ContainerImage {
                                    prefix: "registry.k8s.io/".to_string(),
                                },
                            ],
                        }),
                    },
                ],
            },
        ),
    ]
}

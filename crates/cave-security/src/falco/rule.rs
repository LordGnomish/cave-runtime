// SPDX-License-Identifier: AGPL-3.0-or-later
//! Falco rule types — YAML-deserializable rule, macro, list structures.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Priority
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Hash)]
#[serde(rename_all = "UPPERCASE")]
pub enum Priority {
    Emergency = 0,
    Alert = 1,
    Critical = 2,
    Error = 3,
    Warning = 4,
    Notice = 5,
    Informational = 6,
    Debug = 7,
}

impl Default for Priority {
    fn default() -> Self {
        Priority::Warning
    }
}

impl std::fmt::Display for Priority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Priority::Emergency => "EMERGENCY",
            Priority::Alert => "ALERT",
            Priority::Critical => "CRITICAL",
            Priority::Error => "ERROR",
            Priority::Warning => "WARNING",
            Priority::Notice => "NOTICE",
            Priority::Informational => "INFORMATIONAL",
            Priority::Debug => "DEBUG",
        };
        write!(f, "{s}")
    }
}

// ---------------------------------------------------------------------------
// Source
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    #[default]
    Syscall,
    K8sAudit,
    AwsCloudtrail,
    GcpAudit,
    Custom(String),
}

// ---------------------------------------------------------------------------
// Exceptions
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Exception {
    pub name: String,
    #[serde(default)]
    pub fields: Vec<String>,
    #[serde(default)]
    pub comps: Vec<String>,
    #[serde(default)]
    pub values: Vec<serde_json::Value>,
}

// ---------------------------------------------------------------------------
// Raw YAML entry (union of rule / macro / list)
// ---------------------------------------------------------------------------

/// Single YAML document entry — could be a rule, macro, or list.
#[derive(Debug, Clone, Deserialize)]
pub struct RawEntry {
    // Rule fields
    pub rule: Option<String>,
    #[serde(rename = "macro")]
    pub macro_name: Option<String>,
    pub list: Option<String>,

    pub condition: Option<String>,
    pub output: Option<String>,
    #[serde(default)]
    pub priority: Priority,
    #[serde(default)]
    pub source: Source,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub exceptions: Vec<Exception>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub desc: Option<String>,

    // List fields
    #[serde(default)]
    pub items: Vec<serde_yaml::Value>,

    // Override / append semantics
    #[serde(default)]
    pub append: bool,
    pub warn_evttypes: Option<bool>,
}

fn default_true() -> bool {
    true
}

// ---------------------------------------------------------------------------
// Parsed domain types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FalcoRule {
    pub name: String,
    pub condition: String,
    pub output: String,
    pub priority: Priority,
    pub source: Source,
    pub tags: Vec<String>,
    pub exceptions: Vec<Exception>,
    pub enabled: bool,
    pub desc: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FalcoMacro {
    pub name: String,
    pub condition: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FalcoList {
    pub name: String,
    pub items: Vec<String>,
}

// ---------------------------------------------------------------------------
// Parsed ruleset
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RuleSet {
    pub rules: Vec<FalcoRule>,
    pub macros: Vec<FalcoMacro>,
    pub lists: Vec<FalcoList>,
}

impl RuleSet {
    /// Parse a Falco-style YAML string into a RuleSet.
    pub fn from_yaml(yaml: &str) -> anyhow::Result<Self> {
        let entries: Vec<RawEntry> = serde_yaml::from_str(yaml)?;
        let mut rs = RuleSet::default();
        for entry in entries {
            if let Some(name) = entry.rule {
                rs.rules.push(FalcoRule {
                    name,
                    condition: entry.condition.unwrap_or_default(),
                    output: entry.output.unwrap_or_default(),
                    priority: entry.priority,
                    source: entry.source,
                    tags: entry.tags,
                    exceptions: entry.exceptions,
                    enabled: entry.enabled,
                    desc: entry.desc,
                });
            } else if let Some(name) = entry.macro_name {
                rs.macros.push(FalcoMacro {
                    name,
                    condition: entry.condition.unwrap_or_default(),
                });
            } else if let Some(name) = entry.list {
                let items = entry
                    .items
                    .iter()
                    .filter_map(|v| match v {
                        serde_yaml::Value::String(s) => Some(s.clone()),
                        serde_yaml::Value::Number(n) => Some(n.to_string()),
                        _ => None,
                    })
                    .collect();
                rs.lists.push(FalcoList { name, items });
            }
        }
        Ok(rs)
    }

    pub fn merge(&mut self, other: RuleSet) {
        self.rules.extend(other.rules);
        self.macros.extend(other.macros);
        self.lists.extend(other.lists);
    }
}

// ---------------------------------------------------------------------------
// Builtin rules
// ---------------------------------------------------------------------------

pub const BUILTIN_RULES_YAML: &str = r#"
- list: shell_binaries
  items: [bash, sh, zsh, dash, fish, tcsh, ksh]

- list: sensitive_file_names
  items: [/etc/shadow, /etc/passwd, /etc/sudoers, /root/.ssh/authorized_keys]

- macro: spawned_process
  condition: evt.type = execve and evt.dir = >

- macro: container
  condition: container.id != host

- macro: interactive
  condition: proc.name in (shell_binaries)

- rule: Terminal shell in container
  desc: A shell was spawned in a container
  condition: spawned_process and container and interactive
  output: "Shell spawned in container (user=%user.name container=%container.name image=%container.image.repository cmd=%proc.cmdline)"
  priority: WARNING
  tags: [container, shell]

- rule: Write below etc
  desc: An attempt to write to /etc directory
  condition: >
    evt.type in (open, openat, openat2)
    and evt.dir = >
    and fd.name startswith /etc
  output: "File below /etc opened for writing (user=%user.name command=%proc.cmdline file=%fd.name)"
  priority: ERROR
  tags: [filesystem, mitre_persistence]

- rule: Read sensitive file untrusted
  desc: An attempt to read a sensitive file by a non-root user
  condition: >
    evt.type in (open, openat)
    and fd.name in (sensitive_file_names)
    and user.uid != 0
  output: "Sensitive file opened for reading (user=%user.name file=%fd.name command=%proc.cmdline)"
  priority: WARNING
  tags: [filesystem, mitre_credential_access]

- rule: Outbound connection to external network
  desc: A process established an outbound connection to an external IP
  condition: >
    evt.type = connect
    and evt.dir = >
    and fd.sip != 127.0.0.1
    and fd.sip != 0.0.0.0
  output: "Outbound connection (command=%proc.cmdline connection=%fd.name user=%user.name)"
  priority: NOTICE
  tags: [network]

- rule: Privilege escalation via sudo
  desc: A process attempted to escalate privilege via sudo
  condition: spawned_process and proc.name = sudo
  output: "Sudo invoked (user=%user.name parent=%proc.pname cmd=%proc.cmdline)"
  priority: NOTICE
  tags: [privilege_escalation, mitre_privilege_escalation]

- rule: K8s secret accessed
  desc: A secret was accessed in Kubernetes
  condition: >
    evt.type = get
    and k8s.ns.name != kube-system
  output: "K8s secret accessed (ns=%k8s.ns.name pod=%k8s.pod.name user=%user.name)"
  priority: WARNING
  source: k8s_audit
  tags: [k8s, secrets]

- rule: Container running as root
  desc: Container started running as root
  condition: >
    evt.type = execve
    and container
    and user.uid = 0
    and proc.name != init
  output: "Container running as root (container=%container.name image=%container.image.repository cmd=%proc.cmdline)"
  priority: WARNING
  tags: [container, cis]

- rule: Netcat remote code execution
  desc: Netcat or ncat used for potential reverse shell
  condition: spawned_process and proc.name in (nc, ncat, netcat, nmap)
  output: "Netcat invoked (user=%user.name cmd=%proc.cmdline)"
  priority: CRITICAL
  tags: [network, shell, mitre_execution]
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_builtin_rules() {
        let rs = RuleSet::from_yaml(BUILTIN_RULES_YAML).expect("parse builtin rules");
        assert!(!rs.rules.is_empty());
        assert!(!rs.macros.is_empty());
        assert!(!rs.lists.is_empty());
    }

    #[test]
    fn rule_priorities_ordered() {
        assert!(Priority::Emergency < Priority::Debug);
        assert!(Priority::Critical < Priority::Warning);
    }

    #[test]
    fn ruleset_merge() {
        let mut base = RuleSet::from_yaml(BUILTIN_RULES_YAML).unwrap();
        let extra_yaml = r#"
- rule: My custom rule
  condition: evt.type = open
  output: "Open event"
  priority: DEBUG
"#;
        let extra = RuleSet::from_yaml(extra_yaml).unwrap();
        let before = base.rules.len();
        base.merge(extra);
        assert_eq!(base.rules.len(), before + 1);
    }
}

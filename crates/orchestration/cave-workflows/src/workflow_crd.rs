// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Argo Workflows Workflow + WorkflowSpec + Template CRD shapes —
//! `argoproj/argo-workflows v4.0.5`
//! (`pkg/apis/workflow/v1alpha1/workflow_types.go`).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Top-level Workflow CRD.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Workflow {
    pub id: Uuid,
    pub name: String,
    pub namespace: String,
    pub spec: WorkflowSpec,
    pub status: WorkflowStatus,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkflowSpec {
    pub entrypoint: String,
    pub templates: Vec<Template>,
    #[serde(default)]
    pub arguments: Arguments,
    #[serde(default)]
    pub service_account_name: Option<String>,
    #[serde(default)]
    pub on_exit: Option<String>,
    #[serde(default)]
    pub parallelism: Option<u32>,
    #[serde(default)]
    pub workflow_template_ref: Option<WorkflowTemplateRef>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkflowTemplateRef {
    pub name: String,
    #[serde(default)]
    pub cluster_scope: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Arguments {
    #[serde(default)]
    pub parameters: Vec<Parameter>,
    #[serde(default)]
    pub artifacts: Vec<Artifact>,
}

/// Inputs to a template — parameters + artifacts.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Inputs {
    #[serde(default)]
    pub parameters: Vec<Parameter>,
    #[serde(default)]
    pub artifacts: Vec<Artifact>,
}

/// Outputs from a template.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Outputs {
    #[serde(default)]
    pub parameters: Vec<Parameter>,
    #[serde(default)]
    pub artifacts: Vec<Artifact>,
    #[serde(default)]
    pub result: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Parameter {
    pub name: String,
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub default: Option<String>,
    #[serde(default)]
    pub value_from: Option<ParameterValueFrom>,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParameterValueFrom {
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub parameter: Option<String>,
    #[serde(default)]
    pub jq_filter: Option<String>,
    #[serde(default)]
    pub event: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Artifact {
    pub name: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub repository: Option<ArtifactRepository>,
    #[serde(default)]
    pub archive: Option<ArtifactArchive>,
}

/// Argo Workflows supports seven first-class artifact repositories.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum ArtifactRepository {
    S3 { bucket: String, key: String },
    Gcs { bucket: String, key: String },
    Http { url: String },
    Git { repo: String, revision: String, depth: Option<u32> },
    Oss { bucket: String, key: String, endpoint: String },
    Raw { data: String },
    Hdfs { addresses: Vec<String>, path: String },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum ArtifactArchive {
    None,
    Tar { compression_level: Option<u8> },
    Zip,
}

/// Template variants — each maps to an Argo `template:` field.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Template {
    pub name: String,
    #[serde(default)]
    pub inputs: Inputs,
    #[serde(default)]
    pub outputs: Outputs,
    #[serde(flatten)]
    pub body: TemplateBody,
    #[serde(default)]
    pub retry_strategy: Option<RetryStrategy>,
    #[serde(default)]
    pub timeout: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TemplateBody {
    Container(ContainerTemplate),
    Script(ScriptTemplate),
    Resource(ResourceTemplate),
    Suspend(SuspendTemplate),
    Dag(DagTemplate),
    Steps(StepsTemplate),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ContainerTemplate {
    pub image: String,
    #[serde(default)]
    pub command: Vec<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub working_dir: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScriptTemplate {
    pub image: String,
    pub source: String,
    #[serde(default)]
    pub command: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResourceTemplate {
    /// One of: create / apply / delete / replace / patch / get.
    pub action: String,
    pub manifest: String,
    #[serde(default)]
    pub success_condition: Option<String>,
    #[serde(default)]
    pub failure_condition: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SuspendTemplate {
    /// If `Some`, auto-resume after this many seconds.
    #[serde(default)]
    pub duration: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DagTemplate {
    pub tasks: Vec<DagTask>,
    #[serde(default)]
    pub fail_fast: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DagTask {
    pub name: String,
    pub template: String,
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub arguments: Arguments,
    #[serde(default)]
    pub when: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StepsTemplate {
    pub steps: Vec<Vec<WorkflowStep>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkflowStep {
    pub name: String,
    pub template: String,
    #[serde(default)]
    pub arguments: Arguments,
    #[serde(default)]
    pub when: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RetryStrategy {
    /// Maximum number of retries (including initial attempt).
    pub limit: u32,
    /// `Always` / `OnFailure` / `OnError` / `OnTransientError`.
    pub retry_policy: String,
    #[serde(default)]
    pub backoff: Option<RetryBackoff>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RetryBackoff {
    pub duration: String,
    #[serde(default)]
    pub factor: Option<u32>,
    #[serde(default)]
    pub max_duration: Option<String>,
}

/// Workflow run status.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct WorkflowStatus {
    pub phase: WorkflowPhase,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub finished_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub nodes: HashMap<String, NodeStatus>,
}

#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum WorkflowPhase {
    #[default]
    Pending,
    Running,
    Succeeded,
    Failed,
    Error,
    Suspended,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeStatus {
    pub id: String,
    pub template_name: String,
    pub phase: WorkflowPhase,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub finished_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub outputs: Option<Outputs>,
    #[serde(default)]
    pub children: Vec<String>,
}

impl Workflow {
    pub fn new(name: impl Into<String>, namespace: impl Into<String>, spec: WorkflowSpec) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            namespace: namespace.into(),
            spec,
            status: WorkflowStatus::default(),
            created_at: Utc::now(),
        }
    }

    /// Find a template by name.
    pub fn find_template(&self, name: &str) -> Option<&Template> {
        self.spec.templates.iter().find(|t| t.name == name)
    }

    /// Validate the workflow — entrypoint must exist, DAG tasks reference real
    /// templates, dependencies form no cycles.
    pub fn validate(&self) -> Result<(), String> {
        if self.find_template(&self.spec.entrypoint).is_none() {
            return Err(format!(
                "entrypoint `{}` not in templates",
                self.spec.entrypoint
            ));
        }
        for t in &self.spec.templates {
            if let TemplateBody::Dag(dag) = &t.body {
                for task in &dag.tasks {
                    if self.find_template(&task.template).is_none() {
                        return Err(format!(
                            "template `{}`: task `{}` references missing template `{}`",
                            t.name, task.name, task.template
                        ));
                    }
                }
                detect_cycle_in_dag(&dag.tasks)
                    .map_err(|c| format!("template `{}`: cycle in DAG: {:?}", t.name, c))?;
            }
            if let TemplateBody::Steps(steps) = &t.body {
                for group in &steps.steps {
                    for s in group {
                        if self.find_template(&s.template).is_none() {
                            return Err(format!(
                                "template `{}`: step `{}` references missing template `{}`",
                                t.name, s.name, s.template
                            ));
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

/// Kahn toposort on a DAG; returns the cycle vertices if any.
fn detect_cycle_in_dag(tasks: &[DagTask]) -> Result<(), Vec<String>> {
    use std::collections::{HashMap as M, HashSet, VecDeque};
    let names: HashSet<&str> = tasks.iter().map(|t| t.name.as_str()).collect();
    let mut in_deg: M<&str, usize> = tasks.iter().map(|t| (t.name.as_str(), 0)).collect();
    let mut adj: M<&str, Vec<&str>> = tasks.iter().map(|t| (t.name.as_str(), Vec::new())).collect();
    for t in tasks {
        for d in &t.dependencies {
            if let Some(d) = names.get(d.as_str()) {
                adj.get_mut(d).unwrap().push(t.name.as_str());
                *in_deg.get_mut(t.name.as_str()).unwrap() += 1;
            }
        }
    }
    let mut q: VecDeque<&str> = in_deg
        .iter()
        .filter_map(|(k, v)| if *v == 0 { Some(*k) } else { None })
        .collect();
    let mut visited = 0;
    while let Some(n) = q.pop_front() {
        visited += 1;
        for next in &adj[n] {
            let v = in_deg.get_mut(next).unwrap();
            *v -= 1;
            if *v == 0 {
                q.push_back(next);
            }
        }
    }
    if visited == tasks.len() {
        Ok(())
    } else {
        let remaining: Vec<String> = in_deg
            .iter()
            .filter(|(_, v)| **v > 0)
            .map(|(k, _)| k.to_string())
            .collect();
        Err(remaining)
    }
}

/// Public toposort — returns a valid execution order.
pub fn topo_order(tasks: &[DagTask]) -> Result<Vec<String>, Vec<String>> {
    use std::collections::{HashMap as M, HashSet, VecDeque};
    let names: HashSet<&str> = tasks.iter().map(|t| t.name.as_str()).collect();
    let mut in_deg: M<&str, usize> = tasks.iter().map(|t| (t.name.as_str(), 0)).collect();
    let mut adj: M<&str, Vec<&str>> = tasks.iter().map(|t| (t.name.as_str(), Vec::new())).collect();
    for t in tasks {
        for d in &t.dependencies {
            if let Some(d) = names.get(d.as_str()) {
                adj.get_mut(d).unwrap().push(t.name.as_str());
                *in_deg.get_mut(t.name.as_str()).unwrap() += 1;
            }
        }
    }
    let mut q: VecDeque<&str> = in_deg
        .iter()
        .filter_map(|(k, v)| if *v == 0 { Some(*k) } else { None })
        .collect();
    let mut order = Vec::with_capacity(tasks.len());
    while let Some(n) = q.pop_front() {
        order.push(n.to_string());
        let nexts = adj[n].clone();
        for next in nexts {
            let v = in_deg.get_mut(next).unwrap();
            *v -= 1;
            if *v == 0 {
                q.push_back(next);
            }
        }
    }
    if order.len() == tasks.len() {
        Ok(order)
    } else {
        Err(in_deg
            .iter()
            .filter(|(_, v)| **v > 0)
            .map(|(k, _)| k.to_string())
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cont(image: &str, cmd: &str) -> TemplateBody {
        TemplateBody::Container(ContainerTemplate {
            image: image.into(),
            command: vec!["sh".into(), "-c".into()],
            args: vec![cmd.into()],
            env: HashMap::new(),
            working_dir: None,
        })
    }

    fn tpl(name: &str, body: TemplateBody) -> Template {
        Template {
            name: name.into(),
            inputs: Inputs::default(),
            outputs: Outputs::default(),
            body,
            retry_strategy: None,
            timeout: None,
        }
    }

    #[test]
    fn workflow_validates_when_entrypoint_exists() {
        let wf = Workflow::new(
            "hello",
            "argo",
            WorkflowSpec {
                entrypoint: "main".into(),
                templates: vec![tpl("main", cont("alpine", "echo hi"))],
                arguments: Arguments::default(),
                service_account_name: None,
                on_exit: None,
                parallelism: None,
                workflow_template_ref: None,
            },
        );
        assert!(wf.validate().is_ok());
    }

    #[test]
    fn workflow_validation_rejects_missing_entrypoint() {
        let wf = Workflow::new(
            "x",
            "argo",
            WorkflowSpec {
                entrypoint: "missing".into(),
                templates: vec![tpl("only", cont("alpine", "x"))],
                arguments: Arguments::default(),
                service_account_name: None,
                on_exit: None,
                parallelism: None,
                workflow_template_ref: None,
            },
        );
        assert!(wf.validate().is_err());
    }

    #[test]
    fn dag_toposort_orders_dependencies_first() {
        let tasks = vec![
            DagTask {
                name: "c".into(),
                template: "t".into(),
                dependencies: vec!["b".into()],
                arguments: Arguments::default(),
                when: None,
            },
            DagTask {
                name: "a".into(),
                template: "t".into(),
                dependencies: vec![],
                arguments: Arguments::default(),
                when: None,
            },
            DagTask {
                name: "b".into(),
                template: "t".into(),
                dependencies: vec!["a".into()],
                arguments: Arguments::default(),
                when: None,
            },
        ];
        let order = topo_order(&tasks).unwrap();
        assert_eq!(order.iter().position(|x| x == "a"), Some(0));
        let b = order.iter().position(|x| x == "b").unwrap();
        let c = order.iter().position(|x| x == "c").unwrap();
        assert!(b < c);
    }

    #[test]
    fn dag_toposort_rejects_cycle() {
        let tasks = vec![
            DagTask {
                name: "a".into(),
                template: "t".into(),
                dependencies: vec!["b".into()],
                arguments: Arguments::default(),
                when: None,
            },
            DagTask {
                name: "b".into(),
                template: "t".into(),
                dependencies: vec!["a".into()],
                arguments: Arguments::default(),
                when: None,
            },
        ];
        assert!(topo_order(&tasks).is_err());
    }

    #[test]
    fn validation_catches_cycle_in_workflow_dag() {
        let dag = DagTemplate {
            tasks: vec![
                DagTask {
                    name: "x".into(),
                    template: "t".into(),
                    dependencies: vec!["y".into()],
                    arguments: Arguments::default(),
                    when: None,
                },
                DagTask {
                    name: "y".into(),
                    template: "t".into(),
                    dependencies: vec!["x".into()],
                    arguments: Arguments::default(),
                    when: None,
                },
            ],
            fail_fast: None,
        };
        let wf = Workflow::new(
            "w",
            "argo",
            WorkflowSpec {
                entrypoint: "d".into(),
                templates: vec![
                    tpl("t", cont("alpine", "x")),
                    tpl("d", TemplateBody::Dag(dag)),
                ],
                arguments: Arguments::default(),
                service_account_name: None,
                on_exit: None,
                parallelism: None,
                workflow_template_ref: None,
            },
        );
        let err = wf.validate().unwrap_err();
        assert!(err.contains("cycle"));
    }

    #[test]
    fn steps_validation_rejects_missing_template_reference() {
        let steps = StepsTemplate {
            steps: vec![vec![WorkflowStep {
                name: "s1".into(),
                template: "ghost".into(),
                arguments: Arguments::default(),
                when: None,
            }]],
        };
        let wf = Workflow::new(
            "w",
            "argo",
            WorkflowSpec {
                entrypoint: "main".into(),
                templates: vec![tpl("main", TemplateBody::Steps(steps))],
                arguments: Arguments::default(),
                service_account_name: None,
                on_exit: None,
                parallelism: None,
                workflow_template_ref: None,
            },
        );
        assert!(wf.validate().is_err());
    }

    #[test]
    fn artifact_repository_roundtrips_through_serde() {
        let a = Artifact {
            name: "input".into(),
            path: Some("/tmp/x".into()),
            from: None,
            repository: Some(ArtifactRepository::S3 {
                bucket: "b".into(),
                key: "k".into(),
            }),
            archive: Some(ArtifactArchive::Tar {
                compression_level: Some(6),
            }),
        };
        let j = serde_json::to_string(&a).unwrap();
        let back: Artifact = serde_json::from_str(&j).unwrap();
        assert!(matches!(back.repository, Some(ArtifactRepository::S3 { .. })));
    }

    #[test]
    fn all_six_template_variants_construct() {
        let _c = cont("a", "x");
        let _s = TemplateBody::Script(ScriptTemplate {
            image: "alpine".into(),
            source: "echo".into(),
            command: vec!["sh".into()],
        });
        let _r = TemplateBody::Resource(ResourceTemplate {
            action: "create".into(),
            manifest: "kind: Pod".into(),
            success_condition: None,
            failure_condition: None,
        });
        let _u = TemplateBody::Suspend(SuspendTemplate::default());
        let _d = TemplateBody::Dag(DagTemplate {
            tasks: vec![],
            fail_fast: None,
        });
        let _t = TemplateBody::Steps(StepsTemplate { steps: vec![] });
    }

    #[test]
    fn workflow_phase_defaults_to_pending() {
        let s = WorkflowStatus::default();
        assert_eq!(s.phase, WorkflowPhase::Pending);
    }

    #[test]
    fn retry_strategy_with_backoff_roundtrips() {
        let r = RetryStrategy {
            limit: 3,
            retry_policy: "OnFailure".into(),
            backoff: Some(RetryBackoff {
                duration: "1m".into(),
                factor: Some(2),
                max_duration: Some("10m".into()),
            }),
        };
        let j = serde_json::to_string(&r).unwrap();
        let back: RetryStrategy = serde_json::from_str(&j).unwrap();
        assert_eq!(back.limit, 3);
        assert_eq!(back.backoff.unwrap().factor, Some(2));
    }
}

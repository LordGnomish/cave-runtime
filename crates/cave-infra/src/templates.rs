//! Infrastructure modules/templates — reusable resource blueprints.

use crate::resource::{ResourceKind, ResourceSpec};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use dashmap::DashMap;
use crate::error::{InfraError, InfraResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateParam {
    pub name: String,
    pub description: String,
    pub required: bool,
    pub default: Option<serde_json::Value>,
    pub param_type: String, // "string", "number", "boolean"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InfraTemplate {
    pub name: String,
    pub description: String,
    pub version: String,
    pub params: Vec<TemplateParam>,
    /// Template body: list of resource specs with parameter substitution
    pub resources: Vec<TemplateResource>,
    pub author: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateResource {
    pub kind: String,
    pub name_template: String,
    pub provider: String,
    pub properties: HashMap<String, serde_json::Value>,
    pub depends_on: Vec<String>,
}

/// Render a template with the given parameter values.
pub fn render(template: &InfraTemplate, params: &HashMap<String, serde_json::Value>) -> InfraResult<Vec<ResourceSpec>> {
    // Validate required params
    for p in &template.params {
        if p.required && !params.contains_key(&p.name) && p.default.is_none() {
            return Err(InfraError::TemplateRenderError {
                template: template.name.clone(),
                message: format!("missing required param: {}", p.name),
            });
        }
    }

    // Build effective params (defaults + overrides)
    let mut effective: HashMap<String, serde_json::Value> = template.params.iter()
        .filter_map(|p| p.default.as_ref().map(|d| (p.name.clone(), d.clone())))
        .collect();
    effective.extend(params.clone());

    let mut specs = Vec::new();
    for res in &template.resources {
        let name = substitute(&res.name_template, &effective);
        let mut properties = HashMap::new();
        for (k, v) in &res.properties {
            let substituted = substitute_value(v, &effective);
            properties.insert(k.clone(), substituted);
        }
        specs.push(ResourceSpec {
            kind: ResourceKind::from_str(&res.kind),
            name,
            provider: res.provider.clone(),
            properties,
            depends_on: res.depends_on.iter()
                .map(|d| substitute(d, &effective))
                .collect(),
            tags: HashMap::new(),
        });
    }
    Ok(specs)
}

fn substitute(s: &str, params: &HashMap<String, serde_json::Value>) -> String {
    let mut result = s.to_string();
    for (key, val) in params {
        let placeholder = format!("${{{key}}}");
        let replacement = match val {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        result = result.replace(&placeholder, &replacement);
    }
    result
}

fn substitute_value(v: &serde_json::Value, params: &HashMap<String, serde_json::Value>) -> serde_json::Value {
    match v {
        serde_json::Value::String(s) => {
            // Check if this is a param reference like "${param_name}"
            if s.starts_with("${") && s.ends_with('}') {
                let key = &s[2..s.len()-1];
                if let Some(val) = params.get(key) {
                    return val.clone();
                }
            }
            serde_json::Value::String(substitute(s, params))
        }
        other => other.clone(),
    }
}

// ── Built-in templates ────────────────────────────────────────────────────────

pub fn builtin_templates() -> Vec<InfraTemplate> {
    vec![
        web_app_template(),
        database_cluster_template(),
        kubernetes_infra_template(),
        ha_lb_template(),
    ]
}

fn web_app_template() -> InfraTemplate {
    InfraTemplate {
        name: "web-application".into(),
        description: "Web application stack: load balancer + N web servers + database".into(),
        version: "1.0.0".into(),
        author: "cave-infra".into(),
        params: vec![
            TemplateParam {
                name: "app_name".into(),
                description: "Application name".into(),
                required: true,
                default: None,
                param_type: "string".into(),
            },
            TemplateParam {
                name: "web_count".into(),
                description: "Number of web servers".into(),
                required: false,
                default: Some(serde_json::json!(2)),
                param_type: "number".into(),
            },
            TemplateParam {
                name: "web_cpu".into(),
                description: "CPU count per web server".into(),
                required: false,
                default: Some(serde_json::json!(2)),
                param_type: "number".into(),
            },
            TemplateParam {
                name: "provider".into(),
                description: "Infrastructure provider".into(),
                required: false,
                default: Some(serde_json::json!("noop")),
                param_type: "string".into(),
            },
        ],
        resources: vec![
            TemplateResource {
                kind: "Network".into(),
                name_template: "${app_name}-network".into(),
                provider: "${provider}".into(),
                properties: {
                    let mut m = HashMap::new();
                    m.insert("cidr".into(), serde_json::json!("10.0.0.0/16"));
                    m
                },
                depends_on: vec![],
            },
            TemplateResource {
                kind: "LoadBalancer".into(),
                name_template: "${app_name}-lb".into(),
                provider: "${provider}".into(),
                properties: {
                    let mut m = HashMap::new();
                    m.insert("type".into(), serde_json::json!("http"));
                    m
                },
                depends_on: vec!["${app_name}-network".into()],
            },
            TemplateResource {
                kind: "Server".into(),
                name_template: "${app_name}-web-01".into(),
                provider: "${provider}".into(),
                properties: {
                    let mut m = HashMap::new();
                    m.insert("cpu".into(), serde_json::json!("${web_cpu}"));
                    m.insert("memory_gb".into(), serde_json::json!(4));
                    m.insert("os".into(), serde_json::json!("ubuntu-22.04"));
                    m.insert("role".into(), serde_json::json!("web"));
                    m
                },
                depends_on: vec!["${app_name}-network".into()],
            },
            TemplateResource {
                kind: "Database".into(),
                name_template: "${app_name}-db".into(),
                provider: "${provider}".into(),
                properties: {
                    let mut m = HashMap::new();
                    m.insert("engine".into(), serde_json::json!("postgresql"));
                    m.insert("version".into(), serde_json::json!("16"));
                    m.insert("storage_gb".into(), serde_json::json!(100));
                    m
                },
                depends_on: vec!["${app_name}-network".into()],
            },
        ],
    }
}

fn database_cluster_template() -> InfraTemplate {
    InfraTemplate {
        name: "database-cluster".into(),
        description: "Highly available database cluster with primary + replicas".into(),
        version: "1.0.0".into(),
        author: "cave-infra".into(),
        params: vec![
            TemplateParam {
                name: "cluster_name".into(),
                description: "Cluster name".into(),
                required: true,
                default: None,
                param_type: "string".into(),
            },
            TemplateParam {
                name: "engine".into(),
                description: "Database engine".into(),
                required: false,
                default: Some(serde_json::json!("postgresql")),
                param_type: "string".into(),
            },
            TemplateParam {
                name: "storage_gb".into(),
                description: "Storage per node (GB)".into(),
                required: false,
                default: Some(serde_json::json!(500)),
                param_type: "number".into(),
            },
        ],
        resources: vec![
            TemplateResource {
                kind: "Database".into(),
                name_template: "${cluster_name}-primary".into(),
                provider: "noop".into(),
                properties: {
                    let mut m = HashMap::new();
                    m.insert("engine".into(), serde_json::json!("${engine}"));
                    m.insert("role".into(), serde_json::json!("primary"));
                    m.insert("storage_gb".into(), serde_json::json!("${storage_gb}"));
                    m
                },
                depends_on: vec![],
            },
            TemplateResource {
                kind: "Database".into(),
                name_template: "${cluster_name}-replica-01".into(),
                provider: "noop".into(),
                properties: {
                    let mut m = HashMap::new();
                    m.insert("engine".into(), serde_json::json!("${engine}"));
                    m.insert("role".into(), serde_json::json!("replica"));
                    m.insert("primary".into(), serde_json::json!("${cluster_name}-primary"));
                    m.insert("storage_gb".into(), serde_json::json!("${storage_gb}"));
                    m
                },
                depends_on: vec!["${cluster_name}-primary".into()],
            },
        ],
    }
}

fn kubernetes_infra_template() -> InfraTemplate {
    InfraTemplate {
        name: "kubernetes-infra".into(),
        description: "Kubernetes cluster infrastructure: control plane + worker nodes".into(),
        version: "1.0.0".into(),
        author: "cave-infra".into(),
        params: vec![
            TemplateParam { name: "cluster_name".into(), description: "Cluster name".into(), required: true, default: None, param_type: "string".into() },
            TemplateParam { name: "worker_count".into(), description: "Worker node count".into(), required: false, default: Some(serde_json::json!(3)), param_type: "number".into() },
        ],
        resources: vec![
            TemplateResource {
                kind: "KubernetesCluster".into(),
                name_template: "${cluster_name}".into(),
                provider: "noop".into(),
                properties: {
                    let mut m = HashMap::new();
                    m.insert("version".into(), serde_json::json!("1.30"));
                    m.insert("worker_count".into(), serde_json::json!("${worker_count}"));
                    m
                },
                depends_on: vec![],
            },
        ],
    }
}

fn ha_lb_template() -> InfraTemplate {
    InfraTemplate {
        name: "ha-load-balancer".into(),
        description: "High-availability load balancer with health checks".into(),
        version: "1.0.0".into(),
        author: "cave-infra".into(),
        params: vec![
            TemplateParam { name: "name".into(), description: "LB name".into(), required: true, default: None, param_type: "string".into() },
        ],
        resources: vec![
            TemplateResource {
                kind: "IpAddress".into(),
                name_template: "${name}-vip".into(),
                provider: "noop".into(),
                properties: { let mut m = HashMap::new(); m.insert("type".into(), serde_json::json!("public")); m },
                depends_on: vec![],
            },
            TemplateResource {
                kind: "LoadBalancer".into(),
                name_template: "${name}".into(),
                provider: "noop".into(),
                properties: { let mut m = HashMap::new(); m.insert("type".into(), serde_json::json!("tcp")); m.insert("ha".into(), serde_json::json!(true)); m },
                depends_on: vec!["${name}-vip".into()],
            },
        ],
    }
}

// ── Template registry ─────────────────────────────────────────────────────────

pub struct TemplateRegistry {
    templates: DashMap<String, InfraTemplate>,
}

impl TemplateRegistry {
    pub fn new() -> Self {
        let r = Self { templates: DashMap::new() };
        for t in builtin_templates() {
            r.register(t);
        }
        r
    }

    pub fn register(&self, template: InfraTemplate) {
        self.templates.insert(template.name.clone(), template);
    }

    pub fn get(&self, name: &str) -> InfraResult<InfraTemplate> {
        self.templates.get(name).map(|t| t.clone()).ok_or_else(|| InfraError::TemplateNotFound(name.to_string()))
    }

    pub fn list(&self) -> Vec<InfraTemplate> {
        self.templates.iter().map(|e| e.value().clone()).collect()
    }

    pub fn render(&self, name: &str, params: &HashMap<String, serde_json::Value>) -> InfraResult<Vec<ResourceSpec>> {
        let template = self.get(name)?;
        render(&template, params)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_web_app_template() {
        let registry = TemplateRegistry::new();
        let mut params = HashMap::new();
        params.insert("app_name".into(), serde_json::json!("myapp"));
        let specs = registry.render("web-application", &params).unwrap();
        assert!(!specs.is_empty());
        assert!(specs.iter().any(|s| s.name.contains("myapp")));
        assert!(specs.iter().any(|s| s.kind == ResourceKind::LoadBalancer));
        assert!(specs.iter().any(|s| s.kind == ResourceKind::Database));
    }

    #[test]
    fn missing_required_param_fails() {
        let registry = TemplateRegistry::new();
        let result = registry.render("web-application", &HashMap::new());
        assert!(result.is_err());
    }

    #[test]
    fn builtin_templates_list() {
        let registry = TemplateRegistry::new();
        let templates = registry.list();
        assert!(!templates.is_empty());
        let names: Vec<&str> = templates.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"web-application"));
        assert!(names.contains(&"database-cluster"));
    }

    #[test]
    fn database_cluster_render() {
        let registry = TemplateRegistry::new();
        let mut params = HashMap::new();
        params.insert("cluster_name".into(), serde_json::json!("prod"));
        let specs = registry.render("database-cluster", &params).unwrap();
        assert_eq!(specs.len(), 2);
        assert!(specs.iter().any(|s| s.name.contains("primary")));
        assert!(specs.iter().any(|s| s.name.contains("replica")));
    }
}

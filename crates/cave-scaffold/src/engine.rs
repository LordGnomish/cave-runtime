use crate::models::{ScaffoldTemplate, JobStatus};
use std::collections::HashMap;

/// Validate that all required variables are provided
pub fn validate_parameters(template: &ScaffoldTemplate, params: &HashMap<String, String>) -> Vec<String> {
    template.variables.iter()
        .filter(|v| v.required && !params.contains_key(&v.name))
        .map(|v| format!("Missing required variable: {}", v.name))
        .collect()
}

/// Apply default values for missing optional variables
pub fn apply_defaults(template: &ScaffoldTemplate, params: &mut HashMap<String, String>) {
    for var in &template.variables {
        if !params.contains_key(&var.name) {
            if let Some(ref default) = var.default_value {
                params.insert(var.name.clone(), default.clone());
            }
        }
    }
}

/// Render a template string by replacing {{variable}} placeholders
pub fn render_template(template_str: &str, params: &HashMap<String, String>) -> String {
    let mut result = template_str.to_string();
    for (key, value) in params {
        result = result.replace(&format!("{{{{{key}}}}}"), value);
    }
    result
}

/// Count required variables in a template
pub fn required_variable_count(template: &ScaffoldTemplate) -> usize {
    template.variables.iter().filter(|v| v.required).count()
}

/// Filter templates by category tag
pub fn filter_by_tag<'a>(templates: &'a [ScaffoldTemplate], tag: &str) -> Vec<&'a ScaffoldTemplate> {
    templates.iter().filter(|t| t.tags.iter().any(|tg| tg == tag)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ScaffoldTemplate, TemplateCategory, TemplateVariable, VariableType};
    use uuid::Uuid;
    use chrono::Utc;

    fn make_template(vars: Vec<TemplateVariable>, tags: Vec<String>) -> ScaffoldTemplate {
        ScaffoldTemplate {
            id: Uuid::new_v4(),
            name: "test-template".to_string(),
            description: "A test template".to_string(),
            language: "rust".to_string(),
            category: TemplateCategory::Microservice,
            variables: vars,
            created_at: Utc::now(),
            tags,
        }
    }

    fn required_var(name: &str) -> TemplateVariable {
        TemplateVariable {
            name: name.to_string(),
            description: format!("The {}", name),
            var_type: VariableType::String,
            required: true,
            default_value: None,
        }
    }

    fn optional_var(name: &str, default: &str) -> TemplateVariable {
        TemplateVariable {
            name: name.to_string(),
            description: format!("The {}", name),
            var_type: VariableType::String,
            required: false,
            default_value: Some(default.to_string()),
        }
    }

    #[test]
    fn test_validate_parameters_missing_required() {
        let template = make_template(vec![required_var("service_name")], vec![]);
        let params = HashMap::new();
        let errors = validate_parameters(&template, &params);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("service_name"));
    }

    #[test]
    fn test_validate_parameters_all_provided() {
        let template = make_template(vec![required_var("service_name")], vec![]);
        let mut params = HashMap::new();
        params.insert("service_name".to_string(), "my-svc".to_string());
        let errors = validate_parameters(&template, &params);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_apply_defaults_fills_missing() {
        let template = make_template(vec![optional_var("log_level", "info")], vec![]);
        let mut params = HashMap::new();
        apply_defaults(&template, &mut params);
        assert_eq!(params.get("log_level").map(String::as_str), Some("info"));
    }

    #[test]
    fn test_apply_defaults_doesnt_overwrite() {
        let template = make_template(vec![optional_var("log_level", "info")], vec![]);
        let mut params = HashMap::new();
        params.insert("log_level".to_string(), "debug".to_string());
        apply_defaults(&template, &mut params);
        assert_eq!(params.get("log_level").map(String::as_str), Some("debug"));
    }

    #[test]
    fn test_render_template_replaces_vars() {
        let mut params = HashMap::new();
        params.insert("name".to_string(), "World".to_string());
        let result = render_template("Hello {{name}}!", &params);
        assert_eq!(result, "Hello World!");
    }

    #[test]
    fn test_render_template_multiple_vars() {
        let mut params = HashMap::new();
        params.insert("service".to_string(), "api".to_string());
        params.insert("env".to_string(), "prod".to_string());
        let result = render_template("Deploy {{service}} to {{env}}", &params);
        assert_eq!(result, "Deploy api to prod");
    }

    #[test]
    fn test_required_variable_count() {
        let vars = vec![
            required_var("name"),
            required_var("owner"),
            optional_var("description", ""),
        ];
        let template = make_template(vars, vec![]);
        assert_eq!(required_variable_count(&template), 2);
    }

    #[test]
    fn test_filter_by_tag() {
        let t1 = make_template(vec![], vec!["rust".to_string(), "backend".to_string()]);
        let t2 = make_template(vec![], vec!["go".to_string(), "backend".to_string()]);
        let t3 = make_template(vec![], vec!["frontend".to_string()]);
        let templates = vec![t1, t2, t3];
        let results = filter_by_tag(&templates, "backend");
        assert_eq!(results.len(), 2);
    }
}

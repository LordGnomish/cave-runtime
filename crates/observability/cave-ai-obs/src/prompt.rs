// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Prompt template rendering for Langfuse Prompt Management.
//!
//! Templates use `{{variable}}` syntax (mustache-style, single braces pairs).

use crate::trace_models::PromptTemplate;
use std::collections::HashMap;

/// Error returned when a required template variable is missing.
#[derive(Debug, thiserror::Error)]
pub enum PromptError {
    #[error("missing required template variable: {0}")]
    MissingVariable(String),
}

/// Render a prompt template by substituting `{{variable}}` placeholders.
///
/// Returns an error if any declared variable in `tmpl.variables` is not
/// present in `vars`.
pub fn render_template(
    tmpl: &PromptTemplate,
    vars: &HashMap<String, String>,
) -> Result<String, PromptError> {
    // Validate all declared variables are present.
    for var in &tmpl.variables {
        if !vars.contains_key(var.as_str()) {
            return Err(PromptError::MissingVariable(var.clone()));
        }
    }

    let mut result = tmpl.content.clone();
    for (key, value) in vars {
        let placeholder = format!("{{{{{}}}}}", key);
        result = result.replace(&placeholder, value);
    }
    Ok(result)
}

/// Extract variable names from a template content string.
/// Finds all `{{varname}}` occurrences.
pub fn extract_variables(content: &str) -> Vec<String> {
    let mut vars = Vec::new();
    let mut remaining = content;
    while let Some(start) = remaining.find("{{") {
        let after_open = &remaining[start + 2..];
        if let Some(end) = after_open.find("}}") {
            let var_name = after_open[..end].trim().to_string();
            if !var_name.is_empty() && !vars.contains(&var_name) {
                vars.push(var_name);
            }
            remaining = &after_open[end + 2..];
        } else {
            break;
        }
    }
    vars
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trace_models::PromptTemplate;
    use chrono::Utc;
    use uuid::Uuid;

    fn make_tmpl(content: &str, vars: &[&str]) -> PromptTemplate {
        PromptTemplate {
            id: Uuid::new_v4(),
            name: "test".to_string(),
            version: 1,
            content: content.to_string(),
            variables: vars.iter().map(|s| s.to_string()).collect(),
            is_active: true,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn test_render_no_vars() {
        let tmpl = make_tmpl("Hello world!", &[]);
        let result = render_template(&tmpl, &HashMap::new()).unwrap();
        assert_eq!(result, "Hello world!");
    }

    #[test]
    fn test_render_single_var() {
        let tmpl = make_tmpl("Hi {{name}}!", &["name"]);
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "Bob".to_string());
        assert_eq!(render_template(&tmpl, &vars).unwrap(), "Hi Bob!");
    }

    #[test]
    fn test_render_multiple_vars() {
        let tmpl = make_tmpl("{{greeting}}, {{name}}! From {{city}}.", &["greeting", "name", "city"]);
        let mut vars = HashMap::new();
        vars.insert("greeting".to_string(), "Hello".to_string());
        vars.insert("name".to_string(), "Alice".to_string());
        vars.insert("city".to_string(), "Berlin".to_string());
        let rendered = render_template(&tmpl, &vars).unwrap();
        assert_eq!(rendered, "Hello, Alice! From Berlin.");
    }

    #[test]
    fn test_render_repeated_var() {
        let tmpl = make_tmpl("{{x}} and {{x}}", &["x"]);
        let mut vars = HashMap::new();
        vars.insert("x".to_string(), "foo".to_string());
        assert_eq!(render_template(&tmpl, &vars).unwrap(), "foo and foo");
    }

    #[test]
    fn test_extract_variables_basic() {
        let vars = extract_variables("Hello {{name}}, you are in {{city}}.");
        assert_eq!(vars, vec!["name", "city"]);
    }

    #[test]
    fn test_extract_variables_dedup() {
        let vars = extract_variables("{{x}} plus {{x}}");
        assert_eq!(vars, vec!["x"]);
    }

    #[test]
    fn test_extract_variables_empty() {
        let vars = extract_variables("no variables here");
        assert!(vars.is_empty());
    }
}

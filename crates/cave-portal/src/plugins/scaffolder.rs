// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Scaffolder plugin — software template runner.
//!
//! Templates are fixed-shape recipes that the portal runs to produce a new
//! repo / project (e.g., "Rust web service", "Python CLI tool"). This module
//! models the template metadata and parameter validation; actual file
//! generation is out of scope here.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParamKind {
    String,
    Number,
    Bool,
    Choice,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateParam {
    pub name: String,
    pub label: String,
    pub kind: ParamKind,
    pub required: bool,
    pub default: Option<String>,
    pub choices: Vec<String>,
    pub pattern: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Template {
    pub id: String,
    pub title: String,
    pub description: String,
    pub category: String,
    pub language: String,
    pub params: Vec<TemplateParam>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ScaffolderError {
    #[error("unknown template: {0}")]
    UnknownTemplate(String),
    #[error("missing required parameter: {0}")]
    MissingRequired(String),
    #[error("invalid value for {param:?}: {reason}")]
    InvalidValue { param: String, reason: String },
    #[error("unknown parameter: {0}")]
    UnknownParam(String),
    #[error("template id already exists: {0}")]
    Duplicate(String),
}

#[derive(Debug, Default)]
pub struct ScaffolderPlugin {
    templates: Vec<Template>,
}

impl ScaffolderPlugin {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, t: Template) -> Result<(), ScaffolderError> {
        if self.templates.iter().any(|x| x.id == t.id) {
            return Err(ScaffolderError::Duplicate(t.id));
        }
        self.templates.push(t);
        Ok(())
    }

    pub fn list(&self) -> &[Template] {
        &self.templates
    }

    pub fn list_by_category(&self, cat: &str) -> Vec<&Template> {
        self.templates
            .iter()
            .filter(|t| t.category == cat)
            .collect()
    }

    pub fn find(&self, id: &str) -> Option<&Template> {
        self.templates.iter().find(|t| t.id == id)
    }

    pub fn validate_params(
        &self,
        template_id: &str,
        params: &HashMap<String, String>,
    ) -> Result<(), ScaffolderError> {
        let template = self
            .find(template_id)
            .ok_or_else(|| ScaffolderError::UnknownTemplate(template_id.into()))?;

        for tp in &template.params {
            let value = params.get(&tp.name);
            match (value, &tp.default, tp.required) {
                (None, None, true) => {
                    return Err(ScaffolderError::MissingRequired(tp.name.clone()));
                }
                (None, _, _) => continue, // optional or has default
                (Some(v), _, _) => Self::check_value(tp, v)?,
            }
        }

        for k in params.keys() {
            if !template.params.iter().any(|tp| &tp.name == k) {
                return Err(ScaffolderError::UnknownParam(k.clone()));
            }
        }

        Ok(())
    }

    fn check_value(tp: &TemplateParam, v: &str) -> Result<(), ScaffolderError> {
        match tp.kind {
            ParamKind::String => {
                if v.is_empty() {
                    return Err(ScaffolderError::InvalidValue {
                        param: tp.name.clone(),
                        reason: "empty".into(),
                    });
                }
                if let Some(pat) = &tp.pattern {
                    if !v.chars().all(|c| pat.contains(c)) {
                        return Err(ScaffolderError::InvalidValue {
                            param: tp.name.clone(),
                            reason: format!("contains chars outside pattern {pat:?}"),
                        });
                    }
                }
            }
            ParamKind::Number => {
                if v.parse::<i64>().is_err() {
                    return Err(ScaffolderError::InvalidValue {
                        param: tp.name.clone(),
                        reason: "not a number".into(),
                    });
                }
            }
            ParamKind::Bool => {
                if !matches!(v, "true" | "false") {
                    return Err(ScaffolderError::InvalidValue {
                        param: tp.name.clone(),
                        reason: "not bool".into(),
                    });
                }
            }
            ParamKind::Choice => {
                if !tp.choices.iter().any(|c| c == v) {
                    return Err(ScaffolderError::InvalidValue {
                        param: tp.name.clone(),
                        reason: format!("not in choices {:?}", tp.choices),
                    });
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn template(id: &str, params: Vec<TemplateParam>) -> Template {
        Template {
            id: id.into(),
            title: id.into(),
            description: String::new(),
            category: "lib".into(),
            language: "rust".into(),
            params,
        }
    }

    fn req(name: &str, kind: ParamKind) -> TemplateParam {
        TemplateParam {
            name: name.into(),
            label: name.into(),
            kind,
            required: true,
            default: None,
            choices: Vec::new(),
            pattern: None,
        }
    }

    #[test]
    fn register_inserts() {
        let mut s = ScaffolderPlugin::new();
        s.register(template("t1", vec![])).unwrap();
        assert_eq!(s.list().len(), 1);
    }

    #[test]
    fn register_duplicate_rejected() {
        let mut s = ScaffolderPlugin::new();
        s.register(template("t1", vec![])).unwrap();
        let err = s.register(template("t1", vec![])).unwrap_err();
        assert!(matches!(err, ScaffolderError::Duplicate(_)));
    }

    #[test]
    fn list_by_category_filters() {
        let mut s = ScaffolderPlugin::new();
        let mut a = template("a", vec![]);
        a.category = "lib".into();
        let mut b = template("b", vec![]);
        b.category = "service".into();
        s.register(a).unwrap();
        s.register(b).unwrap();
        assert_eq!(s.list_by_category("lib").len(), 1);
        assert_eq!(s.list_by_category("service").len(), 1);
    }

    #[test]
    fn validate_missing_required() {
        let mut s = ScaffolderPlugin::new();
        s.register(template("t1", vec![req("name", ParamKind::String)]))
            .unwrap();
        let p = HashMap::new();
        let err = s.validate_params("t1", &p).unwrap_err();
        assert!(matches!(err, ScaffolderError::MissingRequired(n) if n == "name"));
    }

    #[test]
    fn validate_unknown_template() {
        let s = ScaffolderPlugin::new();
        let p = HashMap::new();
        let err = s.validate_params("ghost", &p).unwrap_err();
        assert!(matches!(err, ScaffolderError::UnknownTemplate(_)));
    }

    #[test]
    fn validate_unknown_param() {
        let mut s = ScaffolderPlugin::new();
        s.register(template("t1", vec![])).unwrap();
        let mut p = HashMap::new();
        p.insert("foo".into(), "bar".into());
        let err = s.validate_params("t1", &p).unwrap_err();
        assert!(matches!(err, ScaffolderError::UnknownParam(n) if n == "foo"));
    }

    #[test]
    fn validate_default_satisfies_required() {
        let mut tp = req("name", ParamKind::String);
        tp.default = Some("default".into());
        let mut s = ScaffolderPlugin::new();
        s.register(template("t1", vec![tp])).unwrap();
        let p = HashMap::new();
        assert!(s.validate_params("t1", &p).is_ok());
    }

    #[test]
    fn validate_optional_param_skipped() {
        let mut tp = req("name", ParamKind::String);
        tp.required = false;
        let mut s = ScaffolderPlugin::new();
        s.register(template("t1", vec![tp])).unwrap();
        let p = HashMap::new();
        assert!(s.validate_params("t1", &p).is_ok());
    }

    #[test]
    fn validate_string_empty_rejected() {
        let mut s = ScaffolderPlugin::new();
        s.register(template("t1", vec![req("name", ParamKind::String)]))
            .unwrap();
        let mut p = HashMap::new();
        p.insert("name".into(), "".into());
        let err = s.validate_params("t1", &p).unwrap_err();
        assert!(matches!(err, ScaffolderError::InvalidValue { .. }));
    }

    #[test]
    fn validate_number_format() {
        let mut s = ScaffolderPlugin::new();
        s.register(template("t1", vec![req("count", ParamKind::Number)]))
            .unwrap();
        let mut p = HashMap::new();
        p.insert("count".into(), "abc".into());
        let err = s.validate_params("t1", &p).unwrap_err();
        assert!(matches!(err, ScaffolderError::InvalidValue { .. }));
    }

    #[test]
    fn validate_number_accepts_negative() {
        let mut s = ScaffolderPlugin::new();
        s.register(template("t1", vec![req("count", ParamKind::Number)]))
            .unwrap();
        let mut p = HashMap::new();
        p.insert("count".into(), "-42".into());
        assert!(s.validate_params("t1", &p).is_ok());
    }

    #[test]
    fn validate_bool_strict() {
        let mut s = ScaffolderPlugin::new();
        s.register(template("t1", vec![req("flag", ParamKind::Bool)]))
            .unwrap();
        let mut p = HashMap::new();
        p.insert("flag".into(), "yes".into());
        let err = s.validate_params("t1", &p).unwrap_err();
        assert!(matches!(err, ScaffolderError::InvalidValue { .. }));
        p.insert("flag".into(), "true".into());
        assert!(s.validate_params("t1", &p).is_ok());
    }

    #[test]
    fn validate_choice_rejects_outside() {
        let mut tp = req("color", ParamKind::Choice);
        tp.choices = vec!["red".into(), "blue".into()];
        let mut s = ScaffolderPlugin::new();
        s.register(template("t1", vec![tp])).unwrap();
        let mut p = HashMap::new();
        p.insert("color".into(), "green".into());
        let err = s.validate_params("t1", &p).unwrap_err();
        assert!(matches!(err, ScaffolderError::InvalidValue { .. }));
    }

    #[test]
    fn validate_choice_accepts_listed() {
        let mut tp = req("color", ParamKind::Choice);
        tp.choices = vec!["red".into(), "blue".into()];
        let mut s = ScaffolderPlugin::new();
        s.register(template("t1", vec![tp])).unwrap();
        let mut p = HashMap::new();
        p.insert("color".into(), "red".into());
        assert!(s.validate_params("t1", &p).is_ok());
    }

    #[test]
    fn validate_pattern_constraint() {
        let mut tp = req("name", ParamKind::String);
        tp.pattern = Some("abcdefghijklmnopqrstuvwxyz-".into());
        let mut s = ScaffolderPlugin::new();
        s.register(template("t1", vec![tp])).unwrap();
        let mut p = HashMap::new();
        p.insert("name".into(), "Acme!".into());
        let err = s.validate_params("t1", &p).unwrap_err();
        assert!(matches!(err, ScaffolderError::InvalidValue { .. }));
        p.insert("name".into(), "acme-svc".into());
        assert!(s.validate_params("t1", &p).is_ok());
    }

    #[test]
    fn template_serializes() {
        let t = template("t1", vec![req("n", ParamKind::String)]);
        let s = serde_json::to_string(&t).unwrap();
        assert!(s.contains("\"id\":\"t1\""));
    }
}
